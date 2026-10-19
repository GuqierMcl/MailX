//! Per-account background workers for all selectable folder headers.
//!
//! Each account gets one task that owns its IMAP connections. Workers never
//! hold the account database lock across network I/O: they read a sync plan,
//! talk to the server unlocked, then store the result in a short transaction.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use ts_rs::TS;

use super::{folders, messages};
use crate::accounts::AccountService;
use crate::accounts::sync_plan::PlanError;
use crate::error::{AppError, ErrorCode};
use crate::imap::{self, ImapError, Session};
use crate::net::NetError;

const MESSAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
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

/// Receives every sync state change, e.g. to forward it to a UI as a hint to refetch.
pub type SyncListener = Arc<dyn Fn(&AccountSyncState) + Send + Sync>;

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
    listener: SyncListener,
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
            change(state);
            state.sequence = state.sequence.saturating_add(1);
            state.clone()
        };
        // The monotonic token lets consumers reject any reordered delivery.
        (self.listener)(&state);
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
    workers: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl SyncManager {
    pub(crate) fn new(
        service: Arc<Mutex<AccountService>>,
        runtime: Handle,
        listener: SyncListener,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                service: Arc::downgrade(&service),
                states: Mutex::new(HashMap::new()),
                listener,
            }),
            runtime,
            workers: Mutex::new(HashMap::new()),
        }
    }

    /// Starts, or restarts with fresh settings, the worker for an account.
    /// An old connection is never reused after the account changes.
    pub(crate) fn restart(&self, account_id: &str) {
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
        workers.insert(
            account_id.to_owned(),
            self.runtime.spawn(run(shared, token)),
        );
    }

    pub(crate) fn states(&self) -> Vec<AccountSyncState> {
        self.shared
            .states
            .lock()
            .map(|states| states.values().cloned().collect())
            .unwrap_or_default()
    }
}

impl Drop for SyncManager {
    fn drop(&mut self) {
        if let Ok(workers) = self.workers.get_mut() {
            workers.drain().for_each(|(_, worker)| worker.abort());
        }
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

async fn run(shared: Arc<Shared>, token: WorkerToken) {
    let mut failures = 0u32;
    let mut last_folder_refresh = None;
    loop {
        shared.begin_round(&token);
        let wait = match sync_once(&shared, &token, &mut last_folder_refresh).await {
            Outcome::Synced => {
                failures = 0;
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                shared.update(&token, |state| {
                    state.phase = SyncPhase::Idle;
                    state.last_synced_at = Some(now);
                    state.failure = None;
                });
                MESSAGE_REFRESH_INTERVAL
            }
            Outcome::Partial(failure) => {
                failures = 0;
                shared.update(&token, |state| {
                    state.phase = SyncPhase::Failed;
                    state.failure = Some(failure);
                });
                MESSAGE_REFRESH_INTERVAL
            }
            Outcome::Transient(failure) => {
                let wait = backoff(failures);
                failures = failures.saturating_add(1);
                shared.update(&token, |state| {
                    state.phase = SyncPhase::Failed;
                    state.failure = Some(failure);
                });
                wait
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
        tokio::time::sleep(wait).await;
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
    sync_targets(shared, token, targets, session, || {
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
        while session.is_none() {
            tokio::time::sleep(backoff(reconnect_failures.saturating_sub(1))).await;
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
        let result = messages::sync_folder(
            shared,
            token,
            &folder_id,
            &remote_name,
            session.as_mut().expect("connected session"),
        )
        .await;
        match result {
            Ok(()) => {
                reconnect_failures = 0;
                shared.folder_update(token, &folder_id, |folder| {
                    folder.phase = FolderSyncPhase::Idle;
                    folder.last_synced_at = Some(
                        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    );
                    folder.failure = None;
                    folder.data_revision = folder.data_revision.saturating_add(1);
                });
            }
            Err(SyncError::Imap(error)) => {
                let usable = error.session_usable();
                let failure = match classify(error) {
                    Outcome::Transient(failure) => failure,
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
        ImapError::Protocol(_) | ImapError::Mailbox(_) | ImapError::Rejected => {
            Outcome::Transient(failure(SyncFailureCode::Protocol, detail, true))
        }
    }
}

fn failure(code: SyncFailureCode, detail: Option<String>, will_retry: bool) -> SyncFailure {
    let message = match code {
        SyncFailureCode::Authentication => "服务器拒绝了登录，请检查用户名和密码或授权码。",
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
        let manager = SyncManager::new(service.clone(), Handle::current(), Arc::new(|_| {}));
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
        for _ in 0..9 {
            tokio::time::advance(MESSAGE_REFRESH_INTERVAL).await;
            assert!(!folders_due(Some(refreshed)));
        }
        tokio::time::advance(MESSAGE_REFRESH_INTERVAL).await;
        assert!(folders_due(Some(refreshed)));
    }

    #[test]
    fn login_and_setup_problems_wait_for_the_user_instead_of_retrying() {
        for error in [
            ImapError::Authentication("NO".into()),
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
