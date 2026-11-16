//! The in-memory table of runs: state, accumulated output, event numbering and
//! retention. Events leave while the table is locked, so their order is their
//! `seq` order; sinks must return quickly (see `events.rs`).
use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use tokio::{sync::oneshot, task::AbortHandle};
use uuid::Uuid;

use super::types::{
    AssistantEvent, AssistantEventKind, ConfirmationRequest, FinishedReason, PendingConfirmation,
    RunKindTag, RunSnapshot, RunState, ToolActivity, ToolActivitySummary, ToolStatus,
};
use crate::{
    AppError, ErrorCode,
    events::{CoreEvent, Events},
};

/// The current time as RFC 3339 UTC with milliseconds.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Runs that may be active at once.
pub(crate) const MAX_ACTIVE_RUNS: usize = 2;
/// Finished runs stay readable this long ...
pub(crate) const KEEP_FINISHED_FOR: Duration = Duration::from_secs(10 * 60);
/// ... and no more than this many of them.
pub(crate) const KEEP_FINISHED_RUNS: usize = 20;

/// The loop's end of a confirmation: `true` approves.
struct Waiting {
    request_id: String,
    answer: oneshot::Sender<bool>,
}

struct Entry {
    snapshot: RunSnapshot,
    waiting: Option<Waiting>,
    abort: Option<AbortHandle>,
    finished_at: Option<Instant>,
    created: u64,
}

#[derive(Default)]
struct Table {
    runs: HashMap<String, Entry>,
    counter: u64,
}

pub(crate) struct Runs {
    table: Mutex<Table>,
    events: Events,
}

impl Runs {
    pub(crate) fn new(events: Events) -> Self {
        Self {
            table: Mutex::default(),
            events,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Applies `kind` to the run and tells the host. `false` when the run is
    /// unknown or already over, so a task that lost a race with `cancel`
    /// cannot change what was reported.
    fn apply(&self, table: &mut Table, run_id: &str, kind: AssistantEventKind) -> bool {
        let Some(entry) = table.runs.get_mut(run_id) else {
            return false;
        };
        if entry.snapshot.state.finished() {
            return false;
        }
        Self::record(&self.events, entry, run_id, kind);
        if entry.snapshot.state.finished() {
            Self::sweep_at(table, Instant::now());
        }
        true
    }

    fn record(events: &Events, entry: &mut Entry, run_id: &str, kind: AssistantEventKind) {
        let snapshot = &mut entry.snapshot;
        snapshot.seq += 1;
        match &kind {
            AssistantEventKind::State { state } => {
                snapshot.state = *state;
                if *state != RunState::AwaitingConfirmation {
                    snapshot.pending = None;
                    entry.waiting = None;
                }
                if state.finished() {
                    snapshot.completed_at = Some(now_rfc3339());
                    entry.finished_at = Some(Instant::now());
                }
            }
            AssistantEventKind::Text { delta } => snapshot.text.push_str(delta),
            AssistantEventKind::Tool {
                call_id,
                name,
                status,
                summary,
            } => {
                let activity = ToolActivity {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    status: *status,
                    summary: summary.clone(),
                };
                match snapshot.tools.iter_mut().find(|t| t.call_id == *call_id) {
                    Some(existing) => *existing = activity,
                    None => snapshot.tools.push(activity),
                }
            }
            AssistantEventKind::Confirm {
                request_id,
                call_id,
                request,
            } => {
                snapshot.pending = Some(PendingConfirmation {
                    request_id: request_id.clone(),
                    call_id: call_id.clone(),
                    request: request.clone(),
                });
            }
            AssistantEventKind::Finished { reason } => snapshot.finished_reason = Some(*reason),
            AssistantEventKind::Failed { code } => snapshot.error = Some(*code),
        }
        events.emit(&CoreEvent::Assistant(AssistantEvent {
            run_id: run_id.to_owned(),
            seq: snapshot.seq,
            kind,
        }));
    }

    /// Registers a new run in `state`. An active state counts against
    /// [`MAX_ACTIVE_RUNS`]; a full table is `AssistantBusy`.
    pub(crate) fn create(
        &self,
        kind: RunKindTag,
        conversation_id: Option<String>,
        state: RunState,
    ) -> Result<String, AppError> {
        let mut table = self.lock();
        Self::sweep_at(&mut table, Instant::now());
        if state.active() {
            let active = table
                .runs
                .values()
                .filter(|entry| entry.snapshot.state.active())
                .count();
            if active >= MAX_ACTIVE_RUNS {
                return Err(AppError::new(ErrorCode::AssistantBusy));
            }
        }
        let run_id = Uuid::new_v4().to_string();
        table.counter += 1;
        let created = table.counter;
        let mut entry = Entry {
            snapshot: RunSnapshot {
                run_id: run_id.clone(),
                kind,
                // Replaced by the first event below.
                state: RunState::Running,
                text: String::new(),
                tools: Vec::new(),
                tools_available: kind == RunKindTag::Chat,
                error: None,
                finished_reason: None,
                completed_at: None,
                seq: 0,
                conversation_id,
                pending: None,
            },
            waiting: None,
            abort: None,
            finished_at: None,
            created,
        };
        Self::record(
            &self.events,
            &mut entry,
            &run_id,
            AssistantEventKind::State { state },
        );
        table.runs.insert(run_id.clone(), entry);
        Ok(run_id)
    }

    /// A run that is already complete: nothing runs, nothing is emitted.
    pub(crate) fn create_completed(
        &self,
        kind: RunKindTag,
        text: String,
        completed_at: String,
    ) -> RunSnapshot {
        let mut table = self.lock();
        Self::sweep_at(&mut table, Instant::now());
        let run_id = Uuid::new_v4().to_string();
        table.counter += 1;
        let snapshot = RunSnapshot {
            run_id: run_id.clone(),
            kind,
            state: RunState::Completed,
            text,
            tools: Vec::new(),
            tools_available: false,
            error: None,
            finished_reason: Some(FinishedReason::Stop),
            completed_at: Some(completed_at),
            seq: 0,
            conversation_id: None,
            pending: None,
        };
        let created = table.counter;
        table.runs.insert(
            run_id,
            Entry {
                snapshot: snapshot.clone(),
                waiting: None,
                abort: None,
                finished_at: Some(Instant::now()),
                created,
            },
        );
        snapshot
    }

    /// Records that the run goes without tools.
    pub(crate) fn set_no_tools(&self, run_id: &str) {
        if let Some(entry) = self.lock().runs.get_mut(run_id) {
            entry.snapshot.tools_available = false;
        }
    }

    pub(crate) fn set_abort(&self, run_id: &str, handle: AbortHandle) {
        if let Some(entry) = self.lock().runs.get_mut(run_id) {
            entry.abort = Some(handle);
        }
    }

    pub(crate) fn snapshot(&self, run_id: &str) -> Result<RunSnapshot, AppError> {
        self.lock()
            .runs
            .get(run_id)
            .map(|entry| entry.snapshot.clone())
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))
    }

    pub(crate) fn text(&self, run_id: &str, delta: String) -> bool {
        let mut table = self.lock();
        self.apply(&mut table, run_id, AssistantEventKind::Text { delta })
    }

    pub(crate) fn tool(
        &self,
        run_id: &str,
        call_id: &str,
        name: &str,
        status: ToolStatus,
        summary: ToolActivitySummary,
    ) -> bool {
        let mut table = self.lock();
        self.apply(
            &mut table,
            run_id,
            AssistantEventKind::Tool {
                call_id: call_id.to_owned(),
                name: name.to_owned(),
                status,
                summary,
            },
        )
    }

    pub(crate) fn state(&self, run_id: &str, state: RunState) -> bool {
        let mut table = self.lock();
        self.apply(&mut table, run_id, AssistantEventKind::State { state })
    }

    pub(crate) fn complete(&self, run_id: &str, reason: FinishedReason) {
        let mut table = self.lock();
        if self.apply(&mut table, run_id, AssistantEventKind::Finished { reason }) {
            self.apply(
                &mut table,
                run_id,
                AssistantEventKind::State {
                    state: RunState::Completed,
                },
            );
        }
    }

    pub(crate) fn fail(&self, run_id: &str, code: ErrorCode) {
        let mut table = self.lock();
        if self.apply(&mut table, run_id, AssistantEventKind::Failed { code }) {
            self.apply(
                &mut table,
                run_id,
                AssistantEventKind::State {
                    state: RunState::Failed,
                },
            );
        }
    }

    /// Pauses the run on a question for the user: the state becomes
    /// `awaitingConfirmation` and the request goes out as an event. Returns
    /// the request id and the channel the answer arrives on, or `None` when
    /// the run is already over.
    pub(crate) fn ask(
        &self,
        run_id: &str,
        call_id: &str,
        request: ConfirmationRequest,
    ) -> Option<(String, oneshot::Receiver<bool>)> {
        let mut table = self.lock();
        let request_id = Uuid::new_v4().to_string();
        if !self.apply(
            &mut table,
            run_id,
            AssistantEventKind::State {
                state: RunState::AwaitingConfirmation,
            },
        ) {
            return None;
        }
        let (answer, receiver) = oneshot::channel();
        // `apply` found the run above and keeps the lock until here.
        table.runs.get_mut(run_id)?.waiting = Some(Waiting {
            request_id: request_id.clone(),
            answer,
        });
        self.apply(
            &mut table,
            run_id,
            AssistantEventKind::Confirm {
                request_id: request_id.clone(),
                call_id: call_id.to_owned(),
                request,
            },
        );
        Some((request_id, receiver))
    }

    /// Delivers the user's answer to the run's pending question and sets it
    /// running again. `NotFound` for an unknown run; `Conflict` when the run
    /// is not waiting, waits on another request, or the question has already
    /// been answered or has expired.
    pub(crate) fn respond(
        &self,
        run_id: &str,
        request_id: &str,
        approved: bool,
    ) -> Result<(), AppError> {
        let mut table = self.lock();
        let entry = table
            .runs
            .get_mut(run_id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        if entry.snapshot.state != RunState::AwaitingConfirmation
            || entry
                .waiting
                .as_ref()
                .is_none_or(|waiting| waiting.request_id != request_id)
        {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let waiting = entry.waiting.take().expect("checked above");
        // The loop stopped listening (it timed out a moment ago).
        if waiting.answer.send(approved).is_err() {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        self.apply(
            &mut table,
            run_id,
            AssistantEventKind::State {
                state: RunState::Running,
            },
        );
        Ok(())
    }

    /// The question went unanswered: drops it and sets the run running again.
    pub(crate) fn expire(&self, run_id: &str, request_id: &str) {
        let mut table = self.lock();
        let waiting = table.runs.get(run_id).is_some_and(|entry| {
            entry
                .waiting
                .as_ref()
                .is_some_and(|waiting| waiting.request_id == request_id)
        });
        if waiting {
            self.apply(
                &mut table,
                run_id,
                AssistantEventKind::State {
                    state: RunState::Running,
                },
            );
        }
    }

    /// Ends an active run that found the user's consent gone.
    pub(crate) fn await_consent(&self, run_id: &str) {
        self.state(run_id, RunState::AwaitingConsent);
    }

    /// Stops the run's task, which closes its connection, and reports
    /// `canceled`. A run that is over stays as it is. Work already handed to
    /// a blocking thread (one tool call) still finishes; its result is
    /// dropped.
    pub(crate) fn cancel(&self, run_id: &str) -> Result<(), AppError> {
        let mut table = self.lock();
        let entry = table
            .runs
            .get_mut(run_id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        if entry.snapshot.state.finished() {
            return Ok(());
        }
        if let Some(handle) = entry.abort.take() {
            handle.abort();
        }
        self.apply(
            &mut table,
            run_id,
            AssistantEventKind::State {
                state: RunState::Canceled,
            },
        );
        Ok(())
    }

    pub(crate) fn cancel_all(&self) {
        let ids: Vec<String> = self
            .lock()
            .runs
            .iter()
            .filter(|(_, entry)| entry.snapshot.state.active())
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let _ = self.cancel(&id);
        }
    }

    fn sweep_at(table: &mut Table, now: Instant) {
        table.runs.retain(|_, entry| {
            entry
                .finished_at
                .is_none_or(|at| now.saturating_duration_since(at) < KEEP_FINISHED_FOR)
        });
        let mut finished: Vec<(u64, String)> = table
            .runs
            .iter()
            .filter(|(_, entry)| entry.finished_at.is_some())
            .map(|(id, entry)| (entry.created, id.clone()))
            .collect();
        if finished.len() > KEEP_FINISHED_RUNS {
            finished.sort_unstable();
            for (_, id) in finished.drain(..finished.len() - KEEP_FINISHED_RUNS) {
                table.runs.remove(&id);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn sweep_for_test(&self, now: Instant) {
        Self::sweep_at(&mut self.lock(), now);
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().runs.len()
    }
}
