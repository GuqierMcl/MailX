//! Per-account background workers for folders and inbox message headers.
//!
//! Each account gets one task that owns its IMAP connections. Workers never
//! hold the account database lock across network I/O: they read a sync plan,
//! talk to the server unlocked, then store the result in a short transaction.

use std::collections::HashMap;
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
use crate::imap::{self, ImapError};
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
    /// Technical reason from the server or network stack, for display only.
    pub detail: Option<String>,
    /// Whether the worker will try again on its own.
    pub will_retry: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AccountSyncState {
    pub account_id: String,
    pub phase: SyncPhase,
    /// RFC 3339 time of the last successful inbox sync in this session.
    pub last_synced_at: Option<String>,
    pub failure: Option<SyncFailure>,
}

/// Receives every sync state change, e.g. to forward it to a UI as a hint to refetch.
pub type SyncListener = Arc<dyn Fn(&AccountSyncState) + Send + Sync>;

pub(super) struct Shared {
    /// Weak so that closing the core releases the store (and its directory
    /// lock) at once; a worker that finds it gone simply stops.
    service: Weak<Mutex<AccountService>>,
    states: Mutex<HashMap<String, AccountSyncState>>,
    listener: SyncListener,
}

impl Shared {
    fn publish(&self, state: AccountSyncState) {
        if let Ok(mut states) = self.states.lock() {
            states.insert(state.account_id.clone(), state.clone());
        }
        (self.listener)(&state);
    }

    fn update(&self, account_id: &str, change: impl FnOnce(&mut AccountSyncState)) {
        let mut state = self
            .states
            .lock()
            .ok()
            .and_then(|states| states.get(account_id).cloned())
            .unwrap_or_else(|| AccountSyncState {
                account_id: account_id.to_owned(),
                phase: SyncPhase::Idle,
                last_synced_at: None,
                failure: None,
            });
        change(&mut state);
        self.publish(state);
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
        let shared = self.shared.clone();
        let id = account_id.to_owned();
        workers.insert(account_id.to_owned(), self.runtime.spawn(run(shared, id)));
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
    /// Retry after backoff.
    Transient(SyncFailure),
    /// Wait for the account to change; retrying cannot help and may lock the account.
    Blocked(SyncFailure),
    /// The account no longer exists.
    Gone,
}

async fn run(shared: Arc<Shared>, account_id: String) {
    let mut failures = 0u32;
    let mut last_folder_refresh = None;
    loop {
        shared.update(&account_id, |state| state.phase = SyncPhase::Syncing);
        let wait = match sync_once(&shared, &account_id, &mut last_folder_refresh).await {
            Outcome::Synced => {
                failures = 0;
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                shared.update(&account_id, |state| {
                    state.phase = SyncPhase::Idle;
                    state.last_synced_at = Some(now);
                    state.failure = None;
                });
                MESSAGE_REFRESH_INTERVAL
            }
            Outcome::Transient(failure) => {
                let wait = backoff(failures);
                failures = failures.saturating_add(1);
                shared.update(&account_id, |state| {
                    state.phase = SyncPhase::Failed;
                    state.failure = Some(failure);
                });
                wait
            }
            Outcome::Blocked(failure) => {
                shared.update(&account_id, |state| {
                    state.phase = SyncPhase::Failed;
                    state.failure = Some(failure);
                });
                return;
            }
            // The account was removed, or the core closed.
            Outcome::Gone => {
                if let Ok(mut states) = shared.states.lock() {
                    states.remove(&account_id);
                }
                return;
            }
        };
        tokio::time::sleep(wait).await;
    }
}

enum Access<T> {
    Done(T),
    /// The core was closed; the worker should stop.
    Closed,
    Failed,
}

async fn with_service<T: Send + 'static>(
    shared: &Arc<Shared>,
    operation: impl FnOnce(&mut AccountService) -> T + Send + 'static,
) -> Access<T> {
    let Some(service) = shared.service.upgrade() else {
        return Access::Closed;
    };
    tokio::task::spawn_blocking(move || match service.lock() {
        Ok(mut service) => Access::Done(operation(&mut service)),
        Err(_) => Access::Failed,
    })
    .await
    .unwrap_or(Access::Failed)
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

pub(super) async fn with_store<T: Send + 'static>(
    shared: &Arc<Shared>,
    operation: impl FnOnce(&mut AccountService) -> Result<T, AppError> + Send + 'static,
) -> Result<T, SyncError> {
    match with_service(shared, operation).await {
        Access::Done(result) => result.map_err(SyncError::Storage),
        Access::Closed => Err(SyncError::Closed),
        Access::Failed => Err(AppError::new(ErrorCode::StorageUnavailable).into()),
    }
}

fn folders_due(last_refresh: Option<Instant>) -> bool {
    last_refresh.is_none_or(|last| last.elapsed() >= FOLDER_REFRESH_INTERVAL)
}

async fn sync_once(
    shared: &Arc<Shared>,
    account_id: &str,
    last_folder_refresh: &mut Option<Instant>,
) -> Outcome {
    let id = account_id.to_owned();
    let plan = match with_service(shared, move |service| service.sync_plan(&id)).await {
        Access::Done(Ok(plan)) => plan,
        Access::Done(Err(PlanError::Account(error))) if error.code == ErrorCode::NotFound => {
            return Outcome::Gone;
        }
        Access::Done(Err(PlanError::CredentialUntrusted)) => {
            return Outcome::Blocked(failure(SyncFailureCode::CredentialUntrusted, None, false));
        }
        Access::Done(Err(PlanError::CredentialMissing)) => {
            return Outcome::Blocked(failure(SyncFailureCode::CredentialMissing, None, false));
        }
        Access::Done(Err(PlanError::Account(error))) => {
            return Outcome::Transient(failure(
                SyncFailureCode::Storage,
                Some(error.message),
                true,
            ));
        }
        Access::Closed => return Outcome::Gone,
        Access::Failed => return Outcome::Transient(failure(SyncFailureCode::Storage, None, true)),
    };

    let mut session = match imap::open_session(&plan.route, &plan.login).await {
        Ok(session) => session,
        Err(error) => return classify(error),
    };
    let result: Result<(), SyncError> = async {
        if folders_due(*last_folder_refresh) {
            let remote = session.list_folders().await?;
            let records = folders::interpret(&remote);
            let id = account_id.to_owned();
            let source = plan.mailbox_source_id.clone();
            if !with_store(shared, move |service| {
                service.store_folders(&id, &source, &records)
            })
            .await?
            {
                return Err(SyncError::Stale);
            }
            *last_folder_refresh = Some(Instant::now());
        }
        messages::sync_inboxes(shared, account_id, &plan.mailbox_source_id, &mut session).await
    }
    .await;
    // Failed or timed-out sessions are dropped immediately. Only a fully
    // synchronized session is still known to be safe for a final LOGOUT.
    if result.is_ok() {
        let _ = session.logout().await;
    }
    match result {
        Ok(()) => Outcome::Synced,
        Err(SyncError::Imap(error)) => classify(error),
        Err(SyncError::Storage(error)) => {
            Outcome::Transient(failure(SyncFailureCode::Storage, Some(error.message), true))
        }
        Err(SyncError::Stale | SyncError::Closed) => Outcome::Gone,
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
        ImapError::Protocol(_) => {
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
}
