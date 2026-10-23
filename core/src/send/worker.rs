//! One task per account sends its queue, strictly one message at a time, each
//! on its own SMTP connection (sync and write-back never share it). The task
//! never holds the store lock across network I/O: it claims an item and builds
//! the login plan under the lock, talks to the server unlocked, and records the
//! result in a short transaction. The route and credentials are those of the
//! moment the attempt starts; the snapshot only fixes the content.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::Clock;
use super::clock::parse_stamp;
use super::schedule::{self, Transition};
use super::types::{OutboxChanged, OutboxError};
use crate::accounts::{AccountService, Next};
use crate::error::{AppError, ErrorCode};
use crate::events::{CoreEvent, Events};
use crate::smtp::{self, DataStarted, SmtpOutcome, Timeouts, TlsSource, plan_failure};
use crate::storage::outbox::Changed;

/// What tests replace: the clock, the SMTP time budgets and the trust store.
#[derive(Clone)]
pub(crate) struct SendConfig {
    pub clock: Clock,
    pub timeouts: Timeouts,
    pub tls: TlsSource,
}

impl Default for SendConfig {
    fn default() -> Self {
        Self {
            clock: Clock::System,
            timeouts: Timeouts::default(),
            tls: TlsSource::Platform,
        }
    }
}

pub(crate) struct Shared {
    service: Weak<Mutex<AccountService>>,
    events: Events,
    pub(crate) config: SendConfig,
}

impl Shared {
    pub(crate) fn announce(&self, changed: Vec<Changed>) {
        for change in changed {
            self.events
                .emit(&CoreEvent::Outbox(OutboxChanged::from(&change.item)));
            if let Some(draft) = change.draft {
                self.events.emit(&CoreEvent::DraftChanged(draft));
            }
        }
    }
}

/// An account's task and the signal that wakes it.
type Worker = (JoinHandle<()>, Arc<Notify>);

pub(crate) struct OutboxManager {
    pub(crate) shared: Arc<Shared>,
    runtime: Handle,
    workers: Mutex<HashMap<String, Worker>>,
}

impl OutboxManager {
    pub(crate) fn new(
        service: &Arc<Mutex<AccountService>>,
        runtime: Handle,
        events: Events,
        config: SendConfig,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                service: Arc::downgrade(service),
                events,
                config,
            }),
            runtime,
            workers: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.shared.config.clock.now()
    }

    /// Starts the account's worker if it is not running, else wakes it.
    pub(crate) fn start(&self, account_id: &str) {
        let Ok(mut workers) = self.workers.lock() else {
            return;
        };
        if let Some((handle, wake)) = workers.get(account_id)
            && !handle.is_finished()
        {
            wake.notify_one();
            return;
        }
        let wake = Arc::new(Notify::new());
        let handle = self.runtime.spawn(run(
            self.shared.clone(),
            account_id.to_owned(),
            wake.clone(),
        ));
        workers.insert(account_id.to_owned(), (handle, wake));
    }

    /// New item, cancel, retry now or account change: look at the queue again.
    pub(crate) fn wake(&self, account_id: &str) {
        self.start(account_id);
    }
}

impl Drop for OutboxManager {
    fn drop(&mut self) {
        if let Ok(workers) = self.workers.get_mut() {
            workers.drain().for_each(|(_, (handle, _))| handle.abort());
        }
    }
}

async fn blocking<T: Send + 'static>(
    service: &Arc<Mutex<AccountService>>,
    work: impl FnOnce(&mut AccountService) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let service = service.clone();
    tokio::task::spawn_blocking(move || {
        let mut service = service
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        work(&mut service)
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal))?
}

async fn run(shared: Arc<Shared>, account_id: String, wake: Arc<Notify>) {
    let clock = shared.config.clock.clone();
    loop {
        let Some(service) = shared.service.upgrade() else {
            return;
        };
        let id = account_id.clone();
        let now = clock.now();
        let step = blocking(&service, move |service| service.outbox_step(&id, now)).await;
        let step = match step {
            Ok(step) => step,
            Err(_) => {
                // The store was busy or unavailable; try again shortly.
                drop(service);
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(5)) => {}
                    () = wake.notified() => {}
                }
                continue;
            }
        };
        shared.announce(step.changed);
        match step.next {
            Next::Gone => return,
            Next::Wait(at) => {
                drop(service);
                let wait = at
                    .map(|at| (at - clock.now()).to_std().unwrap_or(Duration::ZERO))
                    .unwrap_or(clock.poll_cap())
                    .min(clock.poll_cap());
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = wake.notified() => {}
                }
            }
            Next::Job { job, plan } => {
                let item_id = job.item.id.clone();
                let outcome = match plan {
                    Err(error) => Ok(plan_failure(&error)),
                    // The address changed between claiming and planning: the
                    // snapshot's sender no longer matches the account.
                    Ok(plan) if !plan.sender_address.eq_ignore_ascii_case(&job.envelope.from) => {
                        Err(OutboxError::reason("accountChanged"))
                    }
                    Ok(plan) => {
                        let hook = data_started(&shared, &service, item_id.clone());
                        let outcome = smtp::send_with_tls(
                            &plan.route,
                            &plan.login,
                            &job.envelope,
                            &job.mime,
                            &shared.config.timeouts,
                            &shared.config.tls,
                            hook,
                        )
                        .await;
                        Ok(outcome)
                    }
                };
                let first_queued =
                    parse_stamp(&job.item.first_queued_at).unwrap_or_else(|| clock.now());
                record(
                    &shared,
                    &service,
                    &item_id,
                    job.item.attempts,
                    first_queued,
                    outcome,
                )
                .await;
            }
        }
    }
}

/// Persists "the body is about to be written" before the first body byte.
fn data_started(
    shared: &Arc<Shared>,
    service: &Arc<Mutex<AccountService>>,
    id: String,
) -> DataStarted {
    let service = service.clone();
    let clock = shared.config.clock.clone();
    DataStarted::new(move || async move {
        let now = clock.now();
        blocking(&service, move |service| {
            service.outbox_data_started(&id, now)
        })
        .await
        .map_err(|_| ())
    })
}

async fn record(
    shared: &Arc<Shared>,
    service: &Arc<Mutex<AccountService>>,
    id: &str,
    attempts: u32,
    first_queued: DateTime<Utc>,
    outcome: Result<SmtpOutcome, OutboxError>,
) {
    let now = shared.config.clock.now();
    let id = id.to_owned();
    let transition = match outcome {
        Ok(outcome) => schedule::transition(outcome, attempts, first_queued, now),
        Err(error) => Transition::Fail(error),
    };
    let result = blocking(service, move |service| match transition {
        Transition::Requeue { at, error } => service.outbox_requeue(&id, at, &error, now),
        Transition::Fail(error) => service.outbox_fail(&id, &error, now),
        Transition::Unknown(error) => service.outbox_unknown(&id, &error, now),
        Transition::Accept(reply) => service.outbox_accept(&id, &reply, now),
    })
    .await;
    // If this write fails the item stays `sending`; startup recovery settles
    // it (queued or unknown by the body-start mark), never a silent resend.
    if let Ok(Some(changed)) = result {
        shared.announce(vec![changed]);
    }
}
