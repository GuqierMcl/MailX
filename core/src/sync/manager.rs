//! Per-account background workers for all selectable folder headers.
//!
//! Each account gets one task that owns its IMAP connections. Workers never
//! hold the account database lock across network I/O: they read a sync plan,
//! talk to the server unlocked, then store the result in a short transaction.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use ts_rs::TS;

use super::cadence::{Activity, Cadence, FRONT_WAKE_MIN_GAP};
use super::{folders, messages, writeback};
use crate::accounts::AccountService;
use crate::accounts::sync_plan::PlanError;
use crate::error::{AppError, ErrorCode};
use crate::events::{CoreEvent, Events, NEW_MAIL_DETAILS, NewMail, NewMailMessage};
use crate::imap::{self, ImapError, Session};
use crate::net::NetError;

const FOLDER_REFRESH_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// Retry delay after consecutive transient failures: exponential from 1s,
/// capped at 5 minutes, with jitter so accounts do not retry in lockstep
/// (docs/architecture/mail-sync.md §2).
fn backoff(failures: u32) -> Duration {
    let base = Duration::from_secs(1u64 << failures.min(9)).min(Duration::from_secs(300));
    // Up to 25% extra; the clock's sub-second part is random enough for spreading retries.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.subsec_nanos());
    base + base.mul_f64(f64::from(nanos % 1000) / 4000.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SyncPhase {
    Syncing,
    Idle,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SyncFailureCode {
    /// The server rejected the login. Not retried until the account changes.
    Authentication,
    /// Provider security policy; only an explicit refresh/settings change retries.
    ProviderPolicy,
    /// The saved password was confirmed for other settings and needs re-entry.
    CredentialUntrusted,
    CredentialMissing,
    Network,
    Proxy,
    Tls,
    Timeout,
    /// The server lacks something required, such as STARTTLS.
    Unsupported,
    Protocol,
    Storage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SyncFailure {
    pub code: SyncFailureCode,
    pub message: String,
    /// Sanitized protocol or network reason, for display only.
    pub detail: Option<String>,
    /// Whether the worker will try again on its own.
    pub will_retry: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FolderSyncPhase {
    Waiting,
    Syncing,
    Idle,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FolderSyncState {
    pub folder_id: String,
    pub phase: FolderSyncPhase,
    pub last_synced_at: Option<String>,
    pub failure: Option<SyncFailure>,
    pub data_revision: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AccountSyncState {
    pub account_id: String,
    pub mailbox_source_id: Option<String>,
    /// Incremented synchronously whenever account settings restart the worker.
    pub generation: u32,
    /// Event order within the worker generation.
    pub sequence: u32,
    /// Complete LIST commits; consumers refresh the folder tree on change.
    pub folder_revision: u32,
    pub folders: Vec<FolderSyncState>,
    pub phase: SyncPhase,
    /// RFC 3339 time of the last entirely successful round in this session.
    pub last_synced_at: Option<String>,
    pub failure: Option<SyncFailure>,
}

/// What a finished write-back round tells the host. Not part of the sync state:
/// it is a one-off, so a late or missed delivery only means a missed notice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FlagWriteBack {
    pub account_id: String,
    /// Changes the server confirmed; the list's "waiting" markers may clear.
    #[ts(type = "number")]
    pub sent: u32,
    /// Starred changes kept on this device only because the folder cannot store
    /// the flag.
    #[ts(type = "number")]
    pub local_only_stars: u32,
}

/// Scheduled deletions a write-back round carried out or gave back. Lists of
/// the account refetch; a non-zero `given_back` means messages the user
/// deleted are showing again and the host should say so.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessagesDeleted {
    pub account_id: String,
    #[ts(type = "number")]
    pub deleted: u32,
    #[ts(type = "number")]
    pub given_back: u32,
}

#[derive(Clone)]
pub(super) struct WorkerToken {
    pub(super) account_id: String,
    pub(super) source: String,
    revision: u32,
    generation: u32,
}

pub(super) struct Shared {
    service: Weak<Mutex<AccountService>>,
    states: Mutex<HashMap<String, AccountSyncState>>,
    pub(super) events: Events,
}

impl Shared {
    fn active(state: &AccountSyncState, token: &WorkerToken) -> bool {
        state.generation == token.generation
            && state.mailbox_source_id.as_deref() == Some(token.source.as_str())
    }

    fn update(&self, token: &WorkerToken, change: impl FnOnce(&mut AccountSyncState)) {
        let state = {
            let Ok(mut states) = self.states.lock() else {
                return;
            };
            let Some(state) = states.get_mut(&token.account_id) else {
                return;
            };
            if !Self::active(state, token) {
                return;
            }
            // A policy refusal remains visible until an explicit restart. A
            // concurrent header round cannot hide a write-back refusal with Idle.
            let policy = state
                .failure
                .as_ref()
                .filter(|failure| failure.code == SyncFailureCode::ProviderPolicy)
                .cloned();
            change(state);
            if let Some(policy) = policy {
                state.phase = SyncPhase::Failed;
                state.failure = Some(policy);
            }
            state.sequence = state.sequence.saturating_add(1);
            state.clone()
        };
        // The monotonic token lets consumers reject any reordered delivery.
        self.events.emit(&CoreEvent::Sync(state));
    }

    fn policy_blocked(&self, token: &WorkerToken) -> bool {
        self.states.lock().is_ok_and(|states| {
            states.get(&token.account_id).is_some_and(|state| {
                Self::active(state, token)
                    && state
                        .failure
                        .as_ref()
                        .is_some_and(|failure| failure.code == SyncFailureCode::ProviderPolicy)
            })
        })
    }

    fn block_policy(&self, token: &WorkerToken) {
        self.update(token, |state| {
            state.phase = SyncPhase::Failed;
            state.failure = Some(failure(SyncFailureCode::ProviderPolicy, None, false));
        });
    }

    pub(super) fn folder_update(
        &self,
        token: &WorkerToken,
        folder_id: &str,
        change: impl FnOnce(&mut FolderSyncState),
    ) {
        self.update(token, |state| {
            if let Some(folder) = state
                .folders
                .iter_mut()
                .find(|folder| folder.folder_id == folder_id)
            {
                change(folder);
            }
        });
    }

    pub(super) fn writeback_done(&self, token: &WorkerToken, report: &writeback::WriteBackReport) {
        let still_current = self.states.lock().is_ok_and(|states| {
            states
                .get(&token.account_id)
                .is_some_and(|state| Self::active(state, token))
        });
        if !still_current {
            return;
        }
        if report.sent > 0 || report.local_only_stars > 0 {
            self.events.emit(&CoreEvent::FlagWriteBack(FlagWriteBack {
                account_id: token.account_id.clone(),
                sent: report.sent as u32,
                local_only_stars: report.local_only_stars as u32,
            }));
        }
        if report.deleted > 0 || report.deletions_given_back > 0 {
            self.events.emit(&CoreEvent::Deletions(MessagesDeleted {
                account_id: token.account_id.clone(),
                deleted: report.deleted as u32,
                given_back: report.deletions_given_back as u32,
            }));
        }
    }

    /// Tells the host about new mail found in one folder of a current worker.
    pub(super) async fn announce(
        self: &Arc<Self>,
        token: &WorkerToken,
        mut arrivals: Vec<NewMailMessage>,
    ) {
        if arrivals.is_empty() {
            return;
        }
        let id = token.account_id.clone();
        let Ok(account) = with_store(self, token, move |service| service.get(&id)).await else {
            return;
        };
        let count = arrivals.len() as u32;
        arrivals.truncate(NEW_MAIL_DETAILS);
        self.events.emit(&CoreEvent::NewMail(NewMail {
            account_id: token.account_id.clone(),
            account_name: account.display_name,
            count,
            messages: arrivals,
        }));
    }

    fn begin_round(&self, token: &WorkerToken) {
        self.update(token, |state| {
            state.phase = SyncPhase::Syncing;
            state.failure = None;
            for folder in &mut state.folders {
                // Idle describes a successful attempt in this round. Retain
                // last success and cache revisions, but let an early account
                // failure apply to every folder still waiting for its attempt.
                folder.phase = FolderSyncPhase::Waiting;
                folder.failure = None;
            }
        });
    }

    fn reconcile_targets(&self, token: &WorkerToken, targets: &[(String, String)], listed: bool) {
        self.update(token, |state| {
            let mut old: HashMap<_, _> = state
                .folders
                .drain(..)
                .map(|folder| (folder.folder_id.clone(), folder))
                .collect();
            state.folders = targets
                .iter()
                .map(|(id, _)| {
                    old.remove(id).unwrap_or_else(|| FolderSyncState {
                        folder_id: id.clone(),
                        phase: FolderSyncPhase::Waiting,
                        last_synced_at: None,
                        failure: None,
                        data_revision: 0,
                    })
                })
                .collect();
            if listed {
                state.folder_revision = state.folder_revision.saturating_add(1);
            }
        });
    }
}

pub(crate) struct SyncManager {
    shared: Arc<Shared>,
    runtime: Handle,
    /// Serializes registration with removal, including delayed starts from a
    /// create or connection update that committed before the removal.
    retired: Mutex<HashSet<String>>,
    workers: Mutex<HashMap<String, JoinHandle<()>>>,
    /// Ends the wait between rounds early when the user asks for a refresh.
    wakers: Arc<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>>,
    /// One write-back worker per account, beside its sync worker.
    writers: Mutex<HashMap<String, Writer>>,
    /// How the app is being used, as the host last said. Every worker watches
    /// it: coming to the front starts a round at once.
    activity: tokio::sync::watch::Sender<Activity>,
}

/// An account's write-back task and the signal that wakes it.
type Writer = (JoinHandle<()>, Arc<tokio::sync::Notify>);

impl SyncManager {
    pub(crate) fn new(
        service: Arc<Mutex<AccountService>>,
        runtime: Handle,
        events: Events,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                service: Arc::downgrade(&service),
                states: Mutex::new(HashMap::new()),
                events,
            }),
            runtime,
            retired: Mutex::new(HashSet::new()),
            workers: Mutex::new(HashMap::new()),
            wakers: Arc::new(Mutex::new(HashMap::new())),
            writers: Mutex::new(HashMap::new()),
            activity: tokio::sync::watch::channel(Activity::default()).0,
        }
    }

    /// What the workers were last told; in front until a host says otherwise.
    pub(crate) fn activity(&self) -> Activity {
        *self.activity.borrow()
    }

    /// Tells every worker how the app is being used. Only a real change wakes
    /// them, so a host may repeat itself freely.
    pub(crate) fn set_activity(&self, activity: Activity) {
        self.activity.send_if_modified(|current| {
            let changed = *current != activity;
            *current = activity;
            changed
        });
    }

    /// Tells the account's write-back worker that a local change is waiting.
    pub(crate) fn changes_waiting(&self, account_id: &str) {
        if self.shared.states.lock().is_ok_and(|states| {
            states
                .get(account_id)
                .and_then(|state| state.failure.as_ref())
                .is_some_and(|failure| failure.code == SyncFailureCode::ProviderPolicy)
        }) {
            return;
        }
        if let Ok(writers) = self.writers.lock()
            && let Some((_, wake)) = writers.get(account_id)
        {
            wake.notify_one();
        }
    }

    fn start_writer(&self, account_id: &str, token: WorkerToken) {
        let Ok(mut writers) = self.writers.lock() else {
            return;
        };
        if let Some((worker, _)) = writers.remove(account_id) {
            worker.abort();
        }
        let wake = Arc::new(tokio::sync::Notify::new());
        // A change made before the worker existed (or before a restart) is
        // picked up by the first round.
        wake.notify_one();
        let service = self.shared.service.clone();
        let shared = self.shared.clone();
        let sync_wakers = self.wakers.clone();
        let account = account_id.to_owned();
        let signal = wake.clone();
        let handle = self.runtime.spawn(async move {
            let mut failures = 0u32;
            loop {
                signal.notified().await;
                loop {
                    let Some(service) = service.upgrade() else {
                        return;
                    };
                    // A stale worker (account edited, generation replaced) stops.
                    let live = shared.states.lock().is_ok_and(|states| {
                        states
                            .get(&token.account_id)
                            .is_some_and(|state| Shared::active(state, &token))
                    });
                    if !live || shared.policy_blocked(&token) {
                        return;
                    }
                    let end = writeback::run_round(&service, &account).await;
                    if let writeback::RoundEnd::Done(report) | writeback::RoundEnd::Retry(report) =
                        &end
                    {
                        if report.has_news() {
                            shared.writeback_done(&token, report);
                        }
                        // The messages land in Trash and folder counts move;
                        // let the header sync pick that up now.
                        if report.deleted > 0
                            && let Some(sync) = sync_wakers
                                .lock()
                                .ok()
                                .and_then(|wakers| wakers.get(&account).cloned())
                        {
                            sync.notify_one();
                        }
                    }
                    match end {
                        writeback::RoundEnd::Done(report) => {
                            failures = 0;
                            // A deletion still in its undo window wakes the
                            // worker when it falls due.
                            let Some(due) = report.next_deletion_in else {
                                break;
                            };
                            tokio::select! {
                                () = tokio::time::sleep(due) => {}
                                () = signal.notified() => {}
                            }
                        }
                        writeback::RoundEnd::Retry(_) => {
                            let wait = writeback::retry_delay(failures);
                            failures = failures.saturating_add(1);
                            // A new change ends the wait early.
                            tokio::select! {
                                () = tokio::time::sleep(wait) => {}
                                () = signal.notified() => {}
                            }
                        }
                        writeback::RoundEnd::Blocked => break,
                        writeback::RoundEnd::ProviderPolicy => {
                            shared.block_policy(&token);
                            return;
                        }
                    }
                }
            }
        });
        writers.insert(account_id.to_owned(), (handle, wake));
    }

    /// Starts the next round of an account's worker now instead of after its
    /// wait or backoff. A recognized provider policy is explicitly retried by
    /// starting a fresh generation, including flag write-back. Other stopped
    /// login/setup failures still require a connection settings change.
    pub(crate) fn refresh(&self, account_id: &str) -> bool {
        let policy = self.shared.states.lock().is_ok_and(|states| {
            states
                .get(account_id)
                .and_then(|state| state.failure.as_ref())
                .is_some_and(|failure| failure.code == SyncFailureCode::ProviderPolicy)
        });
        if policy {
            // This is the explicit user recovery path. New flags and activity
            // changes never restart a worker stopped by a security policy.
            self.restart(account_id);
            return true;
        }
        let running = self.workers.lock().is_ok_and(|workers| {
            workers
                .get(account_id)
                .is_some_and(|worker| !worker.is_finished())
        });
        let wake = self
            .wakers
            .lock()
            .ok()
            .and_then(|wakers| wakers.get(account_id).cloned());
        match (running, wake) {
            (true, Some(wake)) => {
                wake.notify_one();
                true
            }
            _ => false,
        }
    }

    /// Wakes a running worker for one more round. Unlike `refresh` it never
    /// restarts anything, so a worker stopped by policy or a login failure
    /// stays stopped.
    pub(crate) fn wake(&self, account_id: &str) {
        if let Some(wake) = self
            .wakers
            .lock()
            .ok()
            .and_then(|wakers| wakers.get(account_id).cloned())
        {
            wake.notify_one();
        }
    }

    /// Starts, or restarts with fresh settings, the worker for an account.
    /// An old connection is never reused after the account changes.
    pub(crate) fn restart(&self, account_id: &str) {
        let Ok(retired) = self.retired.lock() else {
            return;
        };
        if retired.contains(account_id) {
            return;
        }
        let Ok(mut workers) = self.workers.lock() else {
            return;
        };
        if let Some(worker) = workers.remove(account_id) {
            worker.abort();
        }
        let Some(service) = self.shared.service.upgrade() else {
            return;
        };
        let Ok(service) = service.lock() else {
            return;
        };
        let Ok(account) = service.sync_identity(account_id) else {
            return;
        };
        let token = {
            let Ok(mut states) = self.shared.states.lock() else {
                return;
            };
            let generation = states
                .get(account_id)
                .map_or(1, |state| state.generation.saturating_add(1));
            states.insert(
                account_id.to_owned(),
                AccountSyncState {
                    account_id: account_id.to_owned(),
                    mailbox_source_id: Some(account.mailbox_source_id.clone()),
                    generation,
                    sequence: 0,
                    folder_revision: 0,
                    folders: Vec::new(),
                    phase: SyncPhase::Syncing,
                    last_synced_at: None,
                    failure: None,
                },
            );
            WorkerToken {
                account_id: account_id.to_owned(),
                source: account.mailbox_source_id,
                revision: account.revision,
                generation,
            }
        };
        drop(service);
        let shared = self.shared.clone();
        let wake = Arc::new(tokio::sync::Notify::new());
        if let Ok(mut wakers) = self.wakers.lock() {
            wakers.insert(account_id.to_owned(), wake.clone());
        }
        let writer_token = token.clone();
        workers.insert(
            account_id.to_owned(),
            self.runtime
                .spawn(run(shared, token, wake, self.activity.subscribe())),
        );
        drop(workers);
        self.start_writer(account_id, writer_token);
    }

    /// Called after releasing the service lock: restart takes workers before
    /// service, while body registration takes service before flights.
    pub(crate) fn retire(&self, account_id: &str) {
        let mut retired = self
            .retired
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        retired.insert(account_id.to_owned());
        if let Some(worker) = self
            .workers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(account_id)
        {
            worker.abort();
        }
        if let Some((worker, _)) = self
            .writers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(account_id)
        {
            worker.abort();
        }
        self.wakers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(account_id);
        self.shared
            .states
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(account_id);
    }

    pub(crate) fn states(&self) -> Vec<AccountSyncState> {
        self.shared
            .states
            .lock()
            .map(|states| states.values().cloned().collect())
            .unwrap_or_default()
    }
}

impl SyncManager {
    /// Stops every sync and write-back task; the core is closing. Returns the
    /// stopped tasks so the caller can wait for them to finish.
    pub(crate) fn shutdown(&self) -> Vec<tokio::task::AbortHandle> {
        let mut stopped = Vec::new();
        if let Ok(mut workers) = self.workers.lock() {
            for (_, worker) in workers.drain() {
                worker.abort();
                stopped.push(worker.abort_handle());
            }
        }
        if let Ok(mut writers) = self.writers.lock() {
            for (_, (worker, _)) in writers.drain() {
                worker.abort();
                stopped.push(worker.abort_handle());
            }
        }
        stopped
    }
}

impl Drop for SyncManager {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

enum Outcome {
    Synced,
    /// All folders attempted; failures remain visible until the next round.
    Partial(SyncFailure),
    /// Retry after backoff.
    Transient(SyncFailure),
    /// Wait for the account to change; retrying cannot help and may lock the account.
    Blocked(SyncFailure),
    /// The account no longer exists.
    Gone,
}

/// What the worker does after a round: wait the schedule's time, or retry a
/// failure after its own backoff.
enum Pause {
    Cadence,
    Backoff(Duration),
}

/// Folder and listing revisions summed: moves whenever a round changed anything
/// a reader could see, so a round that did not move it was a quiet one.
pub(super) fn revision_total(shared: &Shared, token: &WorkerToken) -> u64 {
    shared.states.lock().map_or(0, |states| {
        states
            .get(&token.account_id)
            .filter(|state| Shared::active(state, token))
            .map_or(0, |state| {
                u64::from(state.folder_revision)
                    + state
                        .folders
                        .iter()
                        .map(|folder| u64::from(folder.data_revision))
                        .sum::<u64>()
            })
    })
}

/// Tells the cadence whether a round that just succeeded found anything: it did
/// when the revision total the readers refetch on has moved since `before`.
pub(super) fn record_round(
    cadence: &mut Cadence,
    shared: &Shared,
    token: &WorkerToken,
    before: u64,
) {
    cadence.record(revision_total(shared, token) != before);
}

async fn run(
    shared: Arc<Shared>,
    token: WorkerToken,
    wake: Arc<tokio::sync::Notify>,
    mut activity: tokio::sync::watch::Receiver<Activity>,
) {
    let mut failures = 0u32;
    let mut last_folder_refresh = None;
    let mut memory = messages::FolderMemory::new();
    let mut cadence = Cadence::default();
    loop {
        if shared.policy_blocked(&token) {
            return;
        }
        shared.begin_round(&token);
        let before = revision_total(&shared, &token);
        let pause = match sync_once(&shared, &token, &mut last_folder_refresh, &mut memory).await {
            Outcome::Synced => {
                failures = 0;
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                shared.update(&token, |state| {
                    state.phase = SyncPhase::Idle;
                    state.last_synced_at = Some(now);
                    state.failure = None;
                });
                record_round(&mut cadence, &shared, &token, before);
                Pause::Cadence
            }
            Outcome::Partial(failure) => {
                failures = 0;
                shared.update(&token, |state| {
                    state.phase = SyncPhase::Failed;
                    state.failure = Some(failure);
                });
                // Something is wrong in one folder: not a quiet mailbox, so no backing off.
                cadence.record(true);
                Pause::Cadence
            }
            Outcome::Transient(failure) => {
                let wait = backoff(failures);
                failures = failures.saturating_add(1);
                shared.update(&token, |state| {
                    state.phase = SyncPhase::Failed;
                    state.failure = Some(failure);
                });
                Pause::Backoff(wait)
            }
            Outcome::Blocked(failure) => {
                shared.update(&token, |state| {
                    state.phase = SyncPhase::Failed;
                    state.failure = Some(failure);
                });
                return;
            }
            // The account was removed, or the core closed.
            Outcome::Gone => {
                if let Ok(mut states) = shared.states.lock()
                    && states
                        .get(&token.account_id)
                        .is_some_and(|state| Shared::active(state, &token))
                {
                    states.remove(&token.account_id);
                }
                return;
            }
        };
        if !wait_for_next_round(&pause, &mut cadence, &wake, &mut activity).await {
            return;
        }
    }
}

/// Sleeps until the next round is due. The deadline is measured from when the
/// round ended and re-derived whenever the app's activity changes, so going to
/// the background lengthens the wait that is already running. Ends early for a manual refresh, and for the app coming to the
/// front (which also wipes the background's quiet streak). Returns false when
/// the manager is gone and the worker should stop.
async fn wait_for_next_round(
    pause: &Pause,
    cadence: &mut Cadence,
    wake: &tokio::sync::Notify,
    activity: &mut tokio::sync::watch::Receiver<Activity>,
) -> bool {
    let ended = Instant::now();
    loop {
        let current = *activity.borrow_and_update();
        // A failed round retries on its own backoff whatever the app is doing.
        let wait = match pause {
            Pause::Backoff(wait) => *wait,
            Pause::Cadence => cadence.wait(current),
        };
        tokio::select! {
            () = tokio::time::sleep_until(ended + wait) => return true,
            () = wake.notified() => {
                cadence.reset();
                return true;
            }
            changed = activity.changed() => {
                if changed.is_err() {
                    return false;
                }
                if current != Activity::Active && *activity.borrow() == Activity::Active {
                    cadence.reset();
                    // Right away, but not within seconds of the last round; a manual
                    // refresh during that short hold still goes through at once.
                    tokio::select! {
                        () = tokio::time::sleep_until(ended + FRONT_WAKE_MIN_GAP) => {}
                        () = wake.notified() => {}
                    }
                    return true;
                }
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum SyncError {
    Imap(ImapError),
    Storage(AppError),
    Stale,
    Closed,
}

impl From<ImapError> for SyncError {
    fn from(error: ImapError) -> Self {
        Self::Imap(error)
    }
}

impl From<AppError> for SyncError {
    fn from(error: AppError) -> Self {
        Self::Storage(error)
    }
}

/// Guard inside the blocking closure, after the service lock is acquired.
/// Aborting an async worker does not cancel already queued spawn_blocking work.
pub(super) async fn with_store<T: Send + 'static>(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    operation: impl FnOnce(&mut AccountService) -> Result<T, AppError> + Send + 'static,
) -> Result<T, SyncError> {
    let service = shared.service.upgrade().ok_or(SyncError::Closed)?;
    let shared = shared.clone();
    let token = token.clone();
    tokio::task::spawn_blocking(move || {
        let mut service = service
            .lock()
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
        let account = service.sync_identity(&token.account_id)?;
        if account.mailbox_source_id != token.source || account.revision != token.revision {
            return Err(SyncError::Stale);
        }
        let states = shared
            .states
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        if !states
            .get(&token.account_id)
            .is_some_and(|state| Shared::active(state, &token))
        {
            return Err(SyncError::Stale);
        }
        if states
            .get(&token.account_id)
            .and_then(|state| state.failure.as_ref())
            .is_some_and(|failure| failure.code == SyncFailureCode::ProviderPolicy)
        {
            return Err(SyncError::Imap(ImapError::ProviderPolicy));
        }
        // Keep the generation guard until the short transaction is finished.
        operation(&mut service).map_err(SyncError::Storage)
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal))?
}

fn folders_due(last_refresh: Option<Instant>) -> bool {
    last_refresh.is_none_or(|last| last.elapsed() >= FOLDER_REFRESH_INTERVAL)
}

async fn sync_once(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    last_folder_refresh: &mut Option<Instant>,
    memory: &mut messages::FolderMemory,
) -> Outcome {
    let id = token.account_id.clone();
    let plan = match with_store(shared, token, move |service| Ok(service.sync_plan(&id))).await {
        Ok(Ok(plan)) => plan,
        Ok(Err(PlanError::Account(error))) if error.code == ErrorCode::NotFound => {
            return Outcome::Gone;
        }
        Ok(Err(PlanError::CredentialUntrusted)) => {
            return Outcome::Blocked(failure(SyncFailureCode::CredentialUntrusted, None, false));
        }
        Ok(Err(PlanError::CredentialMissing)) => {
            return Outcome::Blocked(failure(SyncFailureCode::CredentialMissing, None, false));
        }
        Ok(Err(PlanError::Account(error))) => return from_sync_error(error.into()),
        Err(error) => return from_sync_error(error),
    };
    if plan.mailbox_source_id != token.source {
        return Outcome::Gone;
    }
    let id = token.account_id.clone();
    let source = token.source.clone();
    let cached = match with_store(shared, token, move |service| {
        service.sync_folders(&id, &source)
    })
    .await
    {
        Ok(targets) => targets,
        Err(error) => return from_sync_error(error),
    };
    shared.reconcile_targets(token, &cached, false);
    if shared.policy_blocked(token) {
        return classify(ImapError::ProviderPolicy);
    }
    let mut session = match imap::open_session(&plan.route, &plan.login).await {
        Ok(session) => session,
        Err(error) => return classify(error),
    };
    let listed = folders_due(*last_folder_refresh);
    if listed {
        let result = async {
            let remote = session.list_folders().await?;
            let records = folders::interpret(&remote);
            let id = token.account_id.clone();
            let source = token.source.clone();
            if !with_store(shared, token, move |service| {
                service.store_folders(&id, &source, &records)
            })
            .await?
            {
                return Err(SyncError::Stale);
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            return from_sync_error(error);
        }
        *last_folder_refresh = Some(Instant::now());
    }
    let id = token.account_id.clone();
    let source = token.source.clone();
    let targets = match with_store(shared, token, move |service| {
        service.sync_folders(&id, &source)
    })
    .await
    {
        Ok(targets) => targets,
        Err(error) => return from_sync_error(error),
    };
    shared.reconcile_targets(token, &targets, listed);
    sync_targets(shared, token, targets, session, memory, || {
        imap::open_session(&plan.route, &plan.login)
    })
    .await
}

/// Each target is attempted once per round. A poisoned session is dropped,
/// then reconnects (with bounded exponential backoff) resume at the next target.
/// Persistent account outages retain that position rather than restarting INBOX.
async fn sync_targets<F, Fut>(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    targets: Vec<(String, String)>,
    session: Session,
    memory: &mut messages::FolderMemory,
    mut connect: F,
) -> Outcome
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Session, ImapError>>,
{
    let mut session = Some(session);
    let mut reconnect_failures = 0u32;
    let mut failed = None;
    for (folder_id, remote_name) in targets {
        if shared.policy_blocked(token) {
            return classify(ImapError::ProviderPolicy);
        }
        while session.is_none() {
            if shared.policy_blocked(token) {
                return classify(ImapError::ProviderPolicy);
            }
            tokio::time::sleep(backoff(reconnect_failures.saturating_sub(1))).await;
            if shared.policy_blocked(token) {
                return classify(ImapError::ProviderPolicy);
            }
            match connect().await {
                Ok(connected) => {
                    session = Some(connected);
                    shared.update(token, |state| {
                        state.phase = SyncPhase::Syncing;
                        state.failure = failed.clone();
                    });
                }
                Err(error) => match classify(error) {
                    Outcome::Transient(failure) => {
                        reconnect_failures = reconnect_failures.saturating_add(1);
                        shared.update(token, |state| {
                            state.phase = SyncPhase::Failed;
                            state.failure = Some(failure);
                        });
                    }
                    outcome => return outcome,
                },
            }
        }
        shared.folder_update(token, &folder_id, |folder| {
            folder.phase = FolderSyncPhase::Syncing;
            folder.failure = None;
        });
        let mut round = messages::FolderRound::default();
        let result = messages::sync_folder(
            shared,
            token,
            &folder_id,
            &remote_name,
            session.as_mut().expect("connected session"),
            memory,
            &mut round,
        )
        .await;
        let changed = round.changed;
        // Stored arrivals are announced even if the round failed afterwards:
        // the cursor already moved past them, so they never come up again.
        shared.announce(token, round.arrivals).await;
        match result {
            Ok(()) => {
                reconnect_failures = 0;
                shared.folder_update(token, &folder_id, |folder| {
                    folder.phase = FolderSyncPhase::Idle;
                    folder.last_synced_at = Some(
                        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    );
                    folder.failure = None;
                    // Readers refetch on a new revision, so it only moves when data did.
                    if changed {
                        folder.data_revision = folder.data_revision.saturating_add(1);
                    }
                });
            }
            Err(SyncError::Imap(error)) => {
                let usable = error.session_usable();
                let failure = match classify(error) {
                    Outcome::Transient(failure) => failure,
                    Outcome::Blocked(failure) => {
                        shared.folder_update(token, &folder_id, |folder| {
                            folder.phase = FolderSyncPhase::Failed;
                            folder.failure = Some(failure.clone());
                            folder.data_revision = folder.data_revision.saturating_add(1);
                        });
                        return Outcome::Blocked(failure);
                    }
                    outcome => return outcome,
                };
                shared.folder_update(token, &folder_id, |folder| {
                    folder.phase = FolderSyncPhase::Failed;
                    folder.failure = Some(failure.clone());
                    folder.data_revision = folder.data_revision.saturating_add(1);
                });
                failed.get_or_insert(failure);
                if !usable {
                    session = None;
                    reconnect_failures = reconnect_failures.saturating_add(1);
                }
            }
            Err(error) => {
                if let SyncError::Storage(error) = &error {
                    let failure =
                        failure(SyncFailureCode::Storage, Some(error.message.clone()), true);
                    shared.folder_update(token, &folder_id, |folder| {
                        folder.phase = FolderSyncPhase::Failed;
                        folder.failure = Some(failure);
                        folder.data_revision = folder.data_revision.saturating_add(1);
                    });
                }
                return from_sync_error(error);
            }
        }
    }
    if let Some(session) = session {
        let _ = session.logout().await;
    }
    failed.map_or(Outcome::Synced, Outcome::Partial)
}

fn from_sync_error(error: SyncError) -> Outcome {
    match error {
        SyncError::Imap(error) => classify(error),
        SyncError::Storage(error) if error.code == ErrorCode::NotFound => Outcome::Gone,
        SyncError::Storage(error) => {
            Outcome::Transient(failure(SyncFailureCode::Storage, Some(error.message), true))
        }
        SyncError::Stale | SyncError::Closed => Outcome::Gone,
    }
}

fn classify(error: ImapError) -> Outcome {
    let detail = Some(error.to_string());
    match error {
        ImapError::ProviderPolicy => {
            Outcome::Blocked(failure(SyncFailureCode::ProviderPolicy, None, false))
        }
        ImapError::Authentication(_) => {
            Outcome::Blocked(failure(SyncFailureCode::Authentication, detail, false))
        }
        ImapError::Unsupported(_) => {
            Outcome::Blocked(failure(SyncFailureCode::Unsupported, detail, false))
        }
        ImapError::Net(NetError::Proxy(_)) => {
            Outcome::Transient(failure(SyncFailureCode::Proxy, detail, true))
        }
        ImapError::Net(NetError::Tls(_)) => {
            Outcome::Transient(failure(SyncFailureCode::Tls, detail, true))
        }
        ImapError::Net(NetError::Connect(_)) => {
            Outcome::Transient(failure(SyncFailureCode::Network, detail, true))
        }
        ImapError::Net(NetError::Timeout) | ImapError::Timeout => {
            Outcome::Transient(failure(SyncFailureCode::Timeout, detail, true))
        }
        ImapError::Protocol(_)
        | ImapError::Mailbox(_)
        | ImapError::Rejected
        | ImapError::TooLarge => {
            Outcome::Transient(failure(SyncFailureCode::Protocol, detail, true))
        }
    }
}

fn failure(code: SyncFailureCode, detail: Option<String>, will_retry: bool) -> SyncFailure {
    let message = match code {
        SyncFailureCode::Authentication => "服务器拒绝了登录，请检查用户名和密码或授权码。",
        SyncFailureCode::ProviderPolicy => "服务商安全策略拒绝访问，请按服务商指引处理后手动重试。",
        SyncFailureCode::CredentialUntrusted => {
            "服务器或登录身份已改变，请在账号设置中重新输入密码。"
        }
        SyncFailureCode::CredentialMissing => "找不到已保存的密码，请在账号设置中重新输入。",
        SyncFailureCode::Network => "无法连接到邮件服务器。",
        SyncFailureCode::Proxy => "无法通过代理连接到邮件服务器。",
        SyncFailureCode::Tls => "无法与邮件服务器建立安全连接。",
        SyncFailureCode::Timeout => "邮件服务器响应超时。",
        SyncFailureCode::Unsupported => "邮件服务器不支持所需的功能。",
        SyncFailureCode::Protocol => "邮件服务器返回了无法识别的响应。",
        SyncFailureCode::Storage => "无法保存同步结果。",
    };
    SyncFailure {
        code,
        message: message.into(),
        detail,
        will_retry,
    }
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod message_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::sync_plan::tests::config;
    use crate::secrets::test_support::MemorySecrets;
    use crate::secrets::{SecretStore, SecretStoreError, SecretValue};
    use futures_util::FutureExt;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[derive(Default)]
    struct LockableSecrets {
        inner: MemorySecrets,
        locked: AtomicBool,
        reads: AtomicUsize,
    }

    impl SecretStore for LockableSecrets {
        fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.locked.load(Ordering::SeqCst) {
                Err(SecretStoreError::Locked)
            } else {
                self.inner.get(key)
            }
        }

        fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
            self.inner.set(key, value)
        }

        fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
            self.inner.delete(key)
        }
    }

    #[tokio::test]
    async fn retired_account_rejects_delayed_sync_and_writeback_starts() {
        let directory = TempDir::new().unwrap();
        let mut service =
            AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
        let account = service
            .create(
                serde_json::from_value(json!({
                    "requestId": uuid::Uuid::new_v4().to_string(),
                    "config": config(json!({"kind": "direct"}))
                }))
                .unwrap(),
            )
            .unwrap();
        let service = Arc::new(Mutex::new(service));
        let manager = SyncManager::new(
            service.clone(),
            Handle::current(),
            Arc::new(|_: &CoreEvent| {}),
        );
        manager.restart(&account.id);
        assert_eq!(manager.states().len(), 1);
        assert_eq!(manager.writers.lock().unwrap().len(), 1);
        manager.retire(&account.id);
        manager.restart(&account.id);
        manager.changes_waiting(&account.id);
        assert!(!manager.refresh(&account.id));
        assert!(manager.states().is_empty());
        assert!(manager.workers.lock().unwrap().is_empty());
        assert!(manager.writers.lock().unwrap().is_empty());
        assert!(manager.wakers.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn provider_policy_only_explicit_refresh_restarts_and_old_results_cannot_reblock() {
        let directory = TempDir::new().unwrap();
        let mut service =
            AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
        let account = service.create(serde_json::from_value(json!({
            "requestId": uuid::Uuid::new_v4().to_string(), "config": config(json!({"kind":"direct"}))
        })).unwrap()).unwrap();
        let service = Arc::new(Mutex::new(service));
        let manager = SyncManager::new(
            service.clone(),
            Handle::current(),
            Arc::new(|_: &CoreEvent| {}),
        );
        manager.restart(&account.id);
        // No task is polled on this current-thread runtime: use its actual
        // generation and real wake signals without making network connections.
        let state = manager.states().pop().unwrap();
        let old = WorkerToken {
            account_id: account.id.clone(),
            source: account.mailbox_source_id.clone(),
            revision: account.revision,
            generation: state.generation,
        };
        manager.shared.block_policy(&old);
        let signal = manager.writers.lock().unwrap()[&account.id].1.clone();
        assert!(signal.notified().now_or_never().is_some()); // drain initial round
        manager.changes_waiting(&account.id);
        assert!(signal.notified().now_or_never().is_none());
        manager.set_activity(Activity::Background);
        manager.set_activity(Activity::Active);
        manager.shared.update(&old, |state| {
            state.phase = SyncPhase::Idle;
            state.failure = None;
        });
        assert!(manager.shared.policy_blocked(&old));
        let failed = manager.states().pop().unwrap();
        assert_eq!(failed.phase, SyncPhase::Failed);
        assert!(!failed.failure.unwrap().will_retry);
        manager.workers.lock().unwrap()[&account.id].abort();
        let wake = Arc::new(tokio::sync::Notify::new());
        super::run(
            manager.shared.clone(),
            old.clone(),
            wake,
            manager.activity.subscribe(),
        )
        .await;
        assert!(manager.shared.policy_blocked(&old));

        assert!(manager.refresh(&account.id));
        let current = manager.states().pop().unwrap();
        assert!(current.generation > old.generation);
        assert!(current.failure.is_none());
        manager.shared.block_policy(&old);
        assert!(
            manager.states().pop().unwrap().failure.is_none(),
            "late old policy cannot stop a new generation"
        );
        manager.shared.block_policy(&WorkerToken {
            generation: current.generation,
            ..old.clone()
        });
        manager.restart(&account.id); // connection-update recovery uses this same path
        assert!(manager.states().pop().unwrap().failure.is_none());
        manager.shared.block_policy(&old);
        assert!(manager.states().pop().unwrap().failure.is_none());
        manager.shutdown();
    }

    #[tokio::test]
    async fn worker_identity_guards_do_not_read_credentials() {
        let directory = TempDir::new().unwrap();
        let secrets = Arc::new(LockableSecrets::default());
        let mut service = AccountService::open(directory.path(), secrets.clone()).unwrap();
        let account = service
            .create(
                serde_json::from_value(json!({
                    "requestId": uuid::Uuid::new_v4().to_string(),
                    "config": config(json!({"kind": "direct"}))
                }))
                .unwrap(),
            )
            .unwrap();
        let service = Arc::new(Mutex::new(service));
        let manager = SyncManager::new(
            service.clone(),
            Handle::current(),
            Arc::new(|_: &CoreEvent| {}),
        );
        secrets.reads.store(0, Ordering::SeqCst);
        secrets.locked.store(true, Ordering::SeqCst);

        manager.restart(&account.id);
        assert_eq!(secrets.reads.load(Ordering::SeqCst), 0);
        let state = manager
            .states()
            .pop()
            .expect("worker created from metadata");
        // On this current-thread runtime the worker has not yet been polled.
        // Stop it before yielding so the test performs no credential or network work.
        manager
            .workers
            .lock()
            .unwrap()
            .remove(&account.id)
            .unwrap()
            .abort();
        let token = WorkerToken {
            account_id: account.id.clone(),
            source: account.mailbox_source_id.clone(),
            revision: account.revision,
            generation: state.generation,
        };
        let id = account.id;
        let source = account.mailbox_source_id;
        let folders = with_store(&manager.shared, &token, move |service| {
            service.sync_folders(&id, &source)
        })
        .await
        .unwrap();
        assert!(folders.is_empty());
        assert_eq!(secrets.reads.load(Ordering::SeqCst), 0);

        manager
            .shared
            .states
            .lock()
            .unwrap()
            .get_mut(&token.account_id)
            .unwrap()
            .generation += 1;
        assert!(matches!(
            with_store(&manager.shared, &token, |_| Ok(())).await,
            Err(SyncError::Stale)
        ));
        assert_eq!(secrets.reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn inbox_ticks_do_not_repeat_folder_listing_until_ten_minutes() {
        assert!(folders_due(None));
        let refreshed = Instant::now();
        let step = Duration::from_secs(60);
        for _ in 0..9 {
            tokio::time::advance(step).await;
            assert!(!folders_due(Some(refreshed)));
        }
        tokio::time::advance(step).await;
        assert!(folders_due(Some(refreshed)));
    }

    /// Runs `wait_for_next_round` on a paused clock. `Instant` jumps forward as the
    /// runtime idles, so a ten-minute wait costs no real time.
    struct Waiting {
        done: tokio::task::JoinHandle<bool>,
        started: Instant,
        activity: tokio::sync::watch::Sender<Activity>,
        wake: Arc<tokio::sync::Notify>,
    }

    impl Waiting {
        fn start(pause: Pause, mut cadence: Cadence, activity: Activity) -> Self {
            let (sender, mut receiver) = tokio::sync::watch::channel(activity);
            let wake = Arc::new(tokio::sync::Notify::new());
            let task_wake = wake.clone();
            let started = Instant::now();
            let done = tokio::spawn(async move {
                wait_for_next_round(&pause, &mut cadence, &task_wake, &mut receiver).await
            });
            Self {
                done,
                started,
                activity: sender,
                wake,
            }
        }

        /// How long the wait lasted, or `None` if it is still going after `limit`.
        async fn finished_within(&mut self, limit: Duration) -> Option<Duration> {
            tokio::time::timeout(limit, &mut self.done)
                .await
                .ok()
                .map(|result| {
                    assert!(result.unwrap());
                    self.started.elapsed()
                })
        }
    }

    fn quiet_cadence(rounds: u32) -> Cadence {
        let mut cadence = Cadence::default();
        for _ in 0..rounds {
            cadence.record(false);
        }
        cadence
    }

    #[tokio::test(start_paused = true)]
    async fn in_front_the_next_round_comes_after_twenty_seconds() {
        let mut waiting = Waiting::start(Pause::Cadence, Cadence::default(), Activity::Active);
        let waited = waiting.finished_within(Duration::from_secs(60)).await;
        assert_eq!(waited, Some(Duration::from_secs(20)));
    }

    #[tokio::test(start_paused = true)]
    async fn going_to_the_background_lengthens_the_wait_that_is_already_running() {
        let mut waiting = Waiting::start(Pause::Cadence, Cadence::default(), Activity::Active);
        tokio::time::sleep(Duration::from_secs(10)).await;
        waiting.activity.send(Activity::Background).unwrap();
        // The deadline counts from the end of the round: 3 minutes, not 10 s + 3 min and not 20 s.
        assert!(
            waiting
                .finished_within(Duration::from_secs(150))
                .await
                .is_none()
        );
        let waited = waiting.finished_within(Duration::from_secs(60)).await;
        assert_eq!(waited, Some(Duration::from_secs(180)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_quiet_background_waits_longer_each_time_up_to_ten_minutes() {
        for (quiet_rounds, expected) in [(0, 180), (3, 360), (4, 600), (30, 600)] {
            let mut waiting = Waiting::start(
                Pause::Cadence,
                quiet_cadence(quiet_rounds),
                Activity::Background,
            );
            let waited = waiting.finished_within(Duration::from_secs(900)).await;
            assert_eq!(
                waited,
                Some(Duration::from_secs(expected)),
                "{quiet_rounds} quiet rounds"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn out_of_sight_it_keeps_checking_so_mail_behind_a_locked_screen_is_found() {
        // A locked screen is background, not a pause: the round still comes, on the
        // slow rhythm, which is what lets a new-mail notification reach the user.
        let mut waiting = Waiting::start(Pause::Cadence, Cadence::default(), Activity::Active);
        tokio::time::sleep(Duration::from_secs(5)).await;
        waiting.activity.send(Activity::Background).unwrap();
        let waited = waiting.finished_within(Duration::from_secs(600)).await;
        assert_eq!(waited, Some(Duration::from_secs(180)));
    }

    #[tokio::test(start_paused = true)]
    async fn coming_to_the_front_starts_a_round_at_once_instead_of_waiting_out_the_background() {
        // Back after 12 seconds of a ten-minute background wait: no further delay.
        let mut waiting = Waiting::start(Pause::Cadence, quiet_cadence(30), Activity::Background);
        tokio::time::sleep(Duration::from_secs(12)).await;
        waiting.activity.send(Activity::Active).unwrap();
        let waited = waiting.finished_within(Duration::from_secs(1)).await;
        assert_eq!(waited, Some(Duration::from_secs(12)));
    }

    #[tokio::test(start_paused = true)]
    async fn flicking_back_to_the_front_right_after_a_round_waits_for_the_minimum_gap() {
        let mut waiting = Waiting::start(Pause::Cadence, quiet_cadence(30), Activity::Background);
        tokio::time::sleep(Duration::from_secs(3)).await;
        waiting.activity.send(Activity::Active).unwrap();
        assert!(
            waiting
                .finished_within(Duration::from_secs(5))
                .await
                .is_none()
        );
        let waited = waiting.finished_within(Duration::from_secs(10)).await;
        assert_eq!(waited, Some(FRONT_WAKE_MIN_GAP));
    }

    #[tokio::test(start_paused = true)]
    async fn a_manual_refresh_during_that_short_hold_is_not_delayed() {
        let mut waiting = Waiting::start(Pause::Cadence, quiet_cadence(30), Activity::Background);
        tokio::time::sleep(Duration::from_secs(3)).await;
        waiting.activity.send(Activity::Active).unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        waiting.wake.notify_one();
        let waited = waiting.finished_within(Duration::from_secs(1)).await;
        assert_eq!(waited, Some(Duration::from_secs(4)));
    }

    #[tokio::test(start_paused = true)]
    async fn coming_to_the_front_clears_the_quiet_streak_for_the_next_time_in_the_background() {
        let mut cadence = quiet_cadence(30);
        assert_eq!(
            cadence.wait(Activity::Background),
            super::super::cadence::BACKGROUND_CAP
        );
        let (sender, mut receiver) = tokio::sync::watch::channel(Activity::Background);
        let wake = tokio::sync::Notify::new();
        // The pinned future borrows the cadence; the block ends that borrow before it is read again.
        {
            let round = wait_for_next_round(&Pause::Cadence, &mut cadence, &wake, &mut receiver);
            tokio::pin!(round);
            tokio::select! {
                _ = &mut round => panic!("the background wait is minutes long"),
                () = tokio::time::sleep(Duration::from_secs(5)) => {}
            }
            sender.send(Activity::Active).unwrap();
            assert!(round.await);
        }
        // Going back out of sight starts from the short background wait again.
        assert_eq!(
            cadence.wait(Activity::Background),
            super::super::cadence::BACKGROUND_BASE
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_manual_refresh_ends_the_wait_in_any_state() {
        for activity in [Activity::Active, Activity::Background] {
            let mut waiting = Waiting::start(Pause::Cadence, quiet_cadence(30), activity);
            tokio::time::sleep(Duration::from_secs(1)).await;
            waiting.wake.notify_one();
            let waited = waiting.finished_within(Duration::from_secs(1)).await;
            assert_eq!(waited, Some(Duration::from_secs(1)), "{activity:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_round_keeps_its_own_backoff_whatever_the_app_is_doing() {
        for activity in [Activity::Active, Activity::Background] {
            let mut waiting = Waiting::start(
                Pause::Backoff(Duration::from_secs(40)),
                quiet_cadence(30),
                activity,
            );
            let waited = waiting.finished_within(Duration::from_secs(900)).await;
            assert_eq!(waited, Some(Duration::from_secs(40)), "{activity:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_worker_stops_when_the_manager_is_gone() {
        let waiting = Waiting::start(Pause::Cadence, Cadence::default(), Activity::Background);
        let Waiting { done, activity, .. } = {
            tokio::time::sleep(Duration::from_secs(1)).await;
            waiting.activity.send(Activity::Background).ok();
            waiting
        };
        drop(activity);
        assert!(!done.await.unwrap(), "no sender left: the worker must stop");
    }

    #[test]
    fn login_and_setup_problems_wait_for_the_user_instead_of_retrying() {
        for error in [
            ImapError::Authentication("NO".into()),
            ImapError::ProviderPolicy,
            ImapError::Unsupported("STARTTLS".into()),
        ] {
            assert!(matches!(
                classify(error),
                Outcome::Blocked(SyncFailure {
                    will_retry: false,
                    ..
                })
            ));
        }
        for error in [
            ImapError::Net(NetError::Connect("refused".into())),
            ImapError::Net(NetError::Proxy("refused".into())),
            ImapError::Net(NetError::Tls("bad certificate".into())),
            ImapError::Timeout,
        ] {
            assert!(matches!(
                classify(error),
                Outcome::Transient(SyncFailure {
                    will_retry: true,
                    ..
                })
            ));
        }
    }

    #[test]
    fn backoff_grows_from_one_second_to_five_minutes() {
        let within = |failures, low: u64, high: f64| {
            let wait = backoff(failures).as_secs_f64();
            assert!(
                wait >= low as f64 && wait <= high,
                "attempt {failures}: {wait}s"
            );
        };
        within(0, 1, 1.25);
        within(1, 2, 2.5);
        within(5, 32, 40.0);
        within(9, 300, 375.0);
        within(u32::MAX, 300, 375.0);
    }

    #[test]
    fn proxy_failures_are_reported_as_proxy_problems() {
        let Outcome::Transient(failure) = classify(ImapError::Net(NetError::Proxy("407".into())))
        else {
            panic!("proxy failure should be transient");
        };
        assert_eq!(failure.code, SyncFailureCode::Proxy);
        assert!(failure.detail.unwrap().contains("407"));
    }

    #[tokio::test]
    async fn proxy_response_secrets_do_not_reach_sync_state_or_trigger_direct_fallback() {
        use crate::accounts::types::ProxyKind;
        use crate::net::{self, NetRoute, ProxyRoute};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        for (response, detail) in [
            (
                "HTTP/1.1 403 private-password-marker\r\nX-Private: private-header-marker\r\n\r\n",
                "proxy failed: proxy refused the tunnel (HTTP 403)",
            ),
            (
                "HTTP/1.1 407 private-password-marker\r\nX-Private: private-header-marker\r\n\r\n",
                "proxy failed: proxy authentication required or rejected",
            ),
            (
                "HTTP/1.1 200private-password-marker\r\nX-Private: private-header-marker\r\n\r\n",
                "proxy failed: invalid HTTP proxy response",
            ),
        ] {
            let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target_port = target.local_addr().unwrap().port();
            let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy_port = proxy.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                    assert!(request.len() < 4096);
                }
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let route = NetRoute {
                image_proxy_dns: false,
                proxy: Some(ProxyRoute {
                    kind: ProxyKind::HttpConnect,
                    host: "127.0.0.1".into(),
                    port: proxy_port,
                    auth: None,
                }),
            };
            let error = net::connect(&route, "127.0.0.1", target_port)
                .await
                .expect_err("proxy rejection must stop the connection");
            let Outcome::Transient(failure) = classify(error.into()) else {
                panic!("proxy rejection should remain retryable");
            };
            assert_eq!(failure.code, SyncFailureCode::Proxy);
            assert!(failure.will_retry);
            assert_eq!(failure.detail.as_deref(), Some(detail));
            let state_payload = serde_json::to_string(&failure).unwrap();
            assert!(!state_payload.contains("private-password-marker"));
            assert!(!state_payload.contains("private-header-marker"));
            server.await.unwrap();
            assert!(matches!(
                target.into_std().unwrap().accept(),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
            ));
        }
    }
}
