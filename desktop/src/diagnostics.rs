//! Bounded, local storage for typed diagnostics from the desktop host and core.

mod files;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use mailx_core::diagnostics::{DiagnosticContext, DiagnosticEvent, DiagnosticSink};
use serde::Serialize;

use self::files::LogFiles;

const QUEUE_CAPACITY: usize = 256;
const LINE_LIMIT: usize = 2048;
const REPEAT_CAPACITY: usize = 64;
const REPEAT_INTERVAL: Duration = Duration::from_secs(30);
const DISK_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const SHUTDOWN_WAIT: Duration = Duration::from_millis(250);

pub(crate) struct Diagnostics {
    sender: SyncSender<Message>,
    control: Arc<Control>,
    done: Mutex<Receiver<()>>,
}

enum Message {
    Event(DiagnosticEvent),
    Stop,
}

#[derive(Default)]
struct Control {
    stopping: AtomicBool,
    dropped: AtomicU64,
}

impl Diagnostics {
    pub(crate) fn start(data_dir: &Path) -> Self {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let (done_sender, done) = mpsc::channel();
        let control = Arc::new(Control::default());
        let writer_control = control.clone();
        let data_dir = data_dir.to_owned();
        let _ = std::thread::Builder::new()
            .name("mailx-diagnostics".into())
            .spawn(move || {
                let mut writer = Writer::new(&data_dir);
                writer.run(receiver, &writer_control);
                // Release the per-profile lock before signaling completion.
                drop(writer);
                let _ = done_sender.send(());
            });
        Self {
            sender,
            control,
            done: Mutex::new(done),
        }
    }

    pub(crate) fn sink(&self) -> DiagnosticSink {
        let sender = self.sender.clone();
        let control = self.control.clone();
        Arc::new(move |event| {
            if !control.stopping.load(Ordering::Relaxed)
                && matches!(
                    sender.try_send(Message::Event(event)),
                    Err(TrySendError::Full(_))
                )
            {
                control.dropped.fetch_add(1, Ordering::Relaxed);
            }
        })
    }

    pub(crate) fn shutdown(&self) {
        if self.stop()
            && let Ok(done) = self.done.try_lock()
        {
            let _ = done.recv_timeout(SHUTDOWN_WAIT);
        }
    }

    fn stop(&self) -> bool {
        if self.control.stopping.swap(true, Ordering::Relaxed) {
            return false;
        }
        // A full queue already guarantees that the writer can wake up.
        let _ = self.sender.try_send(Message::Stop);
        true
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Writer<'a> {
    data_dir: &'a Path,
    session: String,
    files: Option<LogFiles>,
    reopen_at: Instant,
    repeats: Vec<Repeat>,
}

struct Repeat {
    written_at: Instant,
    event: DiagnosticEvent,
    count: u64,
}

impl<'a> Writer<'a> {
    fn new(data_dir: &'a Path) -> Self {
        Self {
            data_dir,
            session: uuid::Uuid::new_v4().to_string(),
            files: None,
            reopen_at: Instant::now(),
            repeats: Vec::new(),
        }
    }

    fn run(&mut self, events: Receiver<Message>, control: &Control) {
        let mut overflow_at = Instant::now();
        while !control.stopping.load(Ordering::Relaxed) {
            let now = Instant::now();
            self.flush_repeats(now, false);
            let mut overflow_pending = control.dropped.load(Ordering::Relaxed) > 0;
            if overflow_pending && now >= overflow_at {
                self.report_overflow(control);
                overflow_at = now + REPEAT_INTERVAL;
                overflow_pending = control.dropped.load(Ordering::Relaxed) > 0;
            }
            let deadline = self
                .repeats
                .first()
                .map(|item| item.written_at + REPEAT_INTERVAL)
                .into_iter()
                .chain(overflow_pending.then_some(overflow_at))
                .min();
            let message = match deadline {
                Some(deadline) => {
                    events.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                }
                None => events.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match message {
                Ok(Message::Event(event)) => self.record(event, Instant::now()),
                Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        for message in events.try_iter().take(QUEUE_CAPACITY) {
            if let Message::Event(event) = message {
                self.record(event, Instant::now());
            }
        }
        self.flush_repeats(Instant::now(), true);
        self.report_overflow(control);
    }

    fn record(&mut self, event: DiagnosticEvent, now: Instant) {
        self.flush_repeats(now, false);
        if let DiagnosticEvent::SyncRecovery { context } = &event {
            self.flush_account_repeats(context);
        }
        if !matches!(
            event,
            DiagnosticEvent::SyncFailure { .. } | DiagnosticEvent::SyncRetry { .. }
        ) {
            self.write(&event, None);
            return;
        }
        if let Some(repeat) = self
            .repeats
            .iter_mut()
            .find(|repeat| same_retry(&repeat.event, &event))
        {
            repeat.event = event;
            repeat.count = repeat.count.saturating_add(1);
            return;
        }
        if self.repeats.len() == REPEAT_CAPACITY {
            let repeat = self.repeats.remove(0);
            self.report_repeat(&repeat);
        }
        self.write(&event, None);
        self.repeats.push(Repeat {
            written_at: now,
            event,
            count: 0,
        });
    }

    fn flush_account_repeats(&mut self, context: &DiagnosticContext) {
        let repeats: Vec<_> = self
            .repeats
            .extract_if(.., |repeat| {
                let previous = match &repeat.event {
                    DiagnosticEvent::SyncFailure { context, .. }
                    | DiagnosticEvent::SyncRetry { context, .. } => context,
                    _ => return false,
                };
                previous.account == context.account && previous.backend == context.backend
            })
            .collect();
        for repeat in repeats {
            self.report_repeat(&repeat);
        }
    }

    fn flush_repeats(&mut self, now: Instant, all: bool) {
        while let Some(first) = self.repeats.first() {
            if !all && now.duration_since(first.written_at) < REPEAT_INTERVAL {
                break;
            }
            let first = self.repeats.remove(0);
            self.report_repeat(&first);
        }
    }

    fn report_repeat(&mut self, repeat: &Repeat) {
        if repeat.count > 0 {
            self.write(&repeat.event, Some(repeat.count));
        }
    }

    fn report_overflow(&mut self, control: &Control) {
        let dropped = control.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            self.write(&QueueStatus::QueueOverflow { dropped }, None);
        }
    }

    fn write(&mut self, event: &impl Serialize, repeats: Option<u64>) {
        let now = Instant::now();
        if self.files.is_none() && now >= self.reopen_at {
            self.files = LogFiles::open(self.data_dir).ok();
            self.reopen_at = now + DISK_RETRY_INTERVAL;
        }
        if let Some(files) = self.files.as_mut()
            && let Some(line) = encode(&self.session, event, repeats)
            && files.append(&line).is_err()
        {
            self.files = None;
            self.reopen_at = now + DISK_RETRY_INTERVAL;
        }
    }
}

fn same_retry(first: &DiagnosticEvent, next: &DiagnosticEvent) -> bool {
    match (first, next) {
        (
            DiagnosticEvent::SyncFailure {
                context: a,
                code: a_code,
                will_retry: a_retry,
            },
            DiagnosticEvent::SyncFailure {
                context: b,
                code: b_code,
                will_retry: b_retry,
            },
        ) => {
            a.account == b.account
                && a.backend == b.backend
                && a_code == b_code
                && a_retry == b_retry
        }
        (
            DiagnosticEvent::SyncRetry { context: a, .. },
            DiagnosticEvent::SyncRetry { context: b, .. },
        ) => a.account == b.account && a.backend == b.backend,
        _ => false,
    }
}

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum QueueStatus {
    QueueOverflow { dropped: u64 },
}

fn encode(session: &str, event: &impl Serialize, repeats: Option<u64>) -> Option<Vec<u8>> {
    #[derive(Serialize)]
    struct Record<'a, Event> {
        timestamp: String,
        version: &'static str,
        session: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        repeats: Option<u64>,
        #[serde(flatten)]
        event: &'a Event,
    }

    let mut line = serde_json::to_vec(&Record {
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        version: env!("CARGO_PKG_VERSION"),
        session,
        repeats,
        event,
    })
    .ok()?;
    if line.len() >= LINE_LIMIT {
        return None;
    }
    line.push(b'\n');
    Some(line)
}

#[cfg(test)]
mod tests;
