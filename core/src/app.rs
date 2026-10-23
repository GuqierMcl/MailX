//! The application service every host (desktop, later web and MCP) talks to.
//!
//! It owns the account store and the background sync workers, so account
//! changes and sync stay consistent no matter which host made the change.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::runtime::Handle;

use crate::accounts::AccountService;
use crate::accounts::types::{
    Account, CreateAccountRequest, UpdateConnectionRequest, UpdateProfileRequest,
};
use crate::drafts::types::{
    CreateDraftFromRequest, CreateDraftRequest, Draft, DraftSummary, UpdateDraftRequest,
};
use crate::error::{AppError, ErrorCode};
use crate::events::Events;
use crate::mail::{EmailDetail, EmailListQuery, EmailPage, Folder};
use crate::secrets::SecretStore;
use crate::sync::bodies::BodyManager;
use crate::sync::manager::{AccountSyncState, SyncManager};

mod attachments;
mod avatars;
mod bodies;
#[cfg(any(test, feature = "body-webview-test"))]
mod body_fixture;
mod flags;
pub use attachments::{AttachmentFile, AttachmentInfo};
pub use avatars::{AvatarImage, AvatarKind};
pub use bodies::{BodyBinding, BodyDocument, BodyLinks};
pub use flags::{FlagChange, MessageFlag};

/// Cheap to clone; every clone shares the same store and workers. Methods
/// block on the store for a short transaction and must not be called from
/// inside an async task without `spawn_blocking`.
#[derive(Clone)]
pub struct MailxCore {
    service: Arc<Mutex<AccountService>>,
    sync: Arc<SyncManager>,
    bodies: Arc<BodyManager>,
    avatars: Arc<avatars::AvatarCache>,
    images: Arc<crate::storage::image_cache::ImageCache>,
}

impl MailxCore {
    /// Opens the store in `path` and starts syncing every account on `runtime`.
    /// `events` hears everything the core reports on its own: sync state
    /// changes, write-back results and new mail.
    pub fn open(
        path: &Path,
        secrets: Arc<dyn SecretStore>,
        runtime: Handle,
        events: Events,
    ) -> Result<Self, AppError> {
        // The image cache is an optimization; a directory that cannot be used
        // must not stop mail from working.
        let images = Arc::new(crate::storage::image_cache::ImageCache::open(path)?);
        let service = AccountService::open(path, secrets)?;
        let accounts = service.list()?;
        let service = Arc::new(Mutex::new(service));
        let bodies = Arc::new(BodyManager::new(&service, runtime.clone()));
        let sync = Arc::new(SyncManager::new(service.clone(), runtime, events));
        for account in &accounts {
            sync.restart(&account.id);
        }
        Ok(Self {
            service,
            sync,
            bodies,
            avatars: Arc::default(),
            images,
        })
    }

    fn accounts(&self) -> Result<MutexGuard<'_, AccountService>, AppError> {
        self.service
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>, AppError> {
        self.accounts()?.list()
    }

    pub fn get_account(&self, id: &str) -> Result<Account, AppError> {
        self.accounts()?.get(id)
    }

    /// Creates an account and starts syncing it.
    pub fn create_account(&self, request: CreateAccountRequest) -> Result<Account, AppError> {
        let account = self.accounts()?.create(request)?;
        self.sync.restart(&account.id);
        Ok(account)
    }

    /// Replaces an account's connection and restarts its sync right away, so
    /// no connection made under the old server, route or credentials survives.
    pub fn update_account_connection(
        &self,
        request: UpdateConnectionRequest,
    ) -> Result<Account, AppError> {
        let (account, body_restart) = {
            let mut service = self.accounts()?;
            let account = service.update_connection(request)?;
            let body_restart = self.bodies.restart(&account.id, &mut service);
            (account, body_restart)
        };
        // The configuration is committed even if resetting a body state fails.
        // Always retire the old header connection before returning that error.
        self.sync.restart(&account.id);
        body_restart?;
        Ok(account)
    }

    /// Renames or recolors an account. Sync and body downloads keep running.
    pub fn update_account_profile(
        &self,
        request: UpdateProfileRequest,
    ) -> Result<Account, AppError> {
        self.accounts()?.update_profile(request)
    }

    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>, AppError> {
        self.accounts()?.list_folders(account_id)
    }

    pub fn list_emails(&self, query: &EmailListQuery) -> Result<EmailPage, AppError> {
        self.accounts()?.list_emails(query)
    }

    pub fn get_email(&self, email_id: &str) -> Result<EmailDetail, AppError> {
        self.accounts()?.get_email(email_id)
    }

    /// Explicit user-opened body request; shared work survives individual IPC
    /// waiter cancellation and always honors the account's configured route.
    pub async fn fetch_body(&self, email_id: String) -> Result<EmailDetail, AppError> {
        self.bodies.fetch(email_id).await
    }

    /// Marks a message read or unread, or starred or not. The change shows at
    /// once and is written back to the server in the background; where the
    /// server cannot keep a starred flag it stays on this device, and the host
    /// is told through a `FlagWriteBack` event when that is found out.
    pub fn set_message_flag(
        &self,
        email_id: &str,
        flag: MessageFlag,
        wanted: bool,
    ) -> Result<FlagChange, AppError> {
        let (change, account_id) = {
            let mut service = self.accounts()?;
            let change = service.set_message_flag(email_id, flag.into(), wanted)?;
            let account_id = service.get_email(email_id)?.summary.account_id;
            (change, account_id)
        };
        if change == crate::storage::LocalChange::Queued {
            self.sync.changes_waiting(&account_id);
        }
        Ok(match change {
            crate::storage::LocalChange::Unchanged => FlagChange::Unchanged,
            crate::storage::LocalChange::Queued => FlagChange::Queued,
            crate::storage::LocalChange::LocalOnly => FlagChange::LocalOnly,
        })
    }

    /// Creates a blank or pre-filled draft. `default_account_id` is the
    /// `compose.defaultAccountId` setting, which the host reads from its
    /// settings store; it only applies when the request names no account.
    pub fn create_draft(
        &self,
        request: CreateDraftRequest,
        default_account_id: &str,
    ) -> Result<Draft, AppError> {
        self.accounts()?.create_draft(request, default_account_id)
    }

    /// Creates a reply, reply-all or forward draft from a message whose body
    /// is cached. Never downloads: an uncached body is `BodyNotCached`, and the
    /// caller fetches it with `fetch_body` first.
    pub fn create_draft_from(&self, request: CreateDraftFromRequest) -> Result<Draft, AppError> {
        let source = self.accounts()?.draft_source(&request)?;
        let (detail, account_address, message_id, raw) = match source {
            crate::accounts::DraftSource::Existing(draft) => return Ok(draft),
            crate::accounts::DraftSource::Fresh {
                detail,
                account_address,
                message_id,
                raw,
            } => (detail, account_address, message_id, raw),
        };
        // The raw message is read and parsed without holding the store lock.
        let raw = match raw {
            Some((blobs, blob)) => blobs.read(&blob)?,
            None => Vec::new(),
        };
        let headers = crate::drafts::compose::parse_headers(&raw, message_id);
        self.accounts()?.create_draft_from_source(
            &request,
            &detail,
            &account_address,
            &headers,
            crate::accounts::local_offset(),
        )
    }

    pub fn get_draft(&self, id: &str) -> Result<Draft, AppError> {
        self.accounts()?.get_draft(id)
    }

    /// Newest first; optionally only one account's drafts.
    pub fn list_drafts(&self, account_id: Option<&str>) -> Result<Vec<DraftSummary>, AppError> {
        self.accounts()?.list_drafts(account_id)
    }

    pub fn update_draft(&self, request: UpdateDraftRequest) -> Result<Draft, AppError> {
        self.accounts()?.update_draft(request)
    }

    pub fn delete_draft(&self, id: &str) -> Result<(), AppError> {
        self.accounts()?.delete_draft(id)
    }

    /// Asks an account's sync worker to run its next round now.
    pub fn refresh_sync(&self, account_id: &str) -> Result<(), AppError> {
        self.accounts()?.get(account_id)?;
        self.sync.refresh(account_id);
        Ok(())
    }

    /// Current sync state of every account that has a worker.
    pub fn sync_states(&self) -> Vec<AccountSyncState> {
        self.sync.states()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use serde_json::json;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::accounts::sync_plan::tests::{config, connection_update};
    use crate::secrets::test_support::MemorySecrets;
    use crate::sync::manager::{SyncFailureCode, SyncPhase};

    /// A local port with nothing listening, so connecting fails immediately.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn open(
        directory: &TempDir,
        runtime: &tokio::runtime::Runtime,
    ) -> (MailxCore, mpsc::Receiver<AccountSyncState>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let events: Events = Arc::new(move |event: &crate::events::CoreEvent| {
            if let crate::events::CoreEvent::Sync(state) = event {
                let _ = sender.lock().unwrap().send(state.clone());
            }
        });
        let core = MailxCore::open(
            directory.path(),
            Arc::new(MemorySecrets::default()),
            runtime.handle().clone(),
            events,
        )
        .unwrap();
        (core, receiver)
    }

    fn next_failure(receiver: &mpsc::Receiver<AccountSyncState>) -> AccountSyncState {
        loop {
            let state = receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("no sync state change");
            if state.phase == SyncPhase::Failed {
                return state;
            }
        }
    }

    fn create(core: &MailxCore, port: u16) -> Account {
        let mut value = config(json!({"kind":"direct"}));
        value["imap"]["host"] = json!("127.0.0.1");
        value["imap"]["port"] = json!(port);
        core.create_account(
            serde_json::from_value(
                json!({"requestId": Uuid::new_v4().to_string(), "config": value}),
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn an_unreachable_server_is_reported_and_retried() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = TempDir::new().unwrap();
        let (core, states) = open(&directory, &runtime);
        let account = create(&core, closed_port());

        let state = next_failure(&states);
        assert_eq!(state.account_id, account.id);
        let failure = state.failure.unwrap();
        assert_eq!(failure.code, SyncFailureCode::Network);
        assert!(failure.will_retry);
        assert_eq!(
            core.sync_states(),
            vec![AccountSyncState {
                failure: Some(failure),
                ..state
            }]
        );
    }

    #[test]
    fn a_changed_server_waits_for_the_password_instead_of_connecting() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = TempDir::new().unwrap();
        let (core, states) = open(&directory, &runtime);
        let account = create(&core, closed_port());
        next_failure(&states);

        let mut value = config(json!({"kind":"direct"}));
        value["imap"]["host"] = json!("127.0.0.2");
        value["imap"]["port"] = json!(closed_port());
        value["authentication"]["password"] = json!({"action":"keep"});
        core.update_account_connection(connection_update(&account.id, account.revision, value))
            .unwrap();

        let failure = next_failure(&states).failure.unwrap();
        assert_eq!(failure.code, SyncFailureCode::CredentialUntrusted);
        assert!(!failure.will_retry);
    }

    #[test]
    fn a_profile_change_keeps_the_running_sync_while_a_connection_change_restarts_it() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = TempDir::new().unwrap();
        let (core, states) = open(&directory, &runtime);
        let account = create(&core, closed_port());
        let generation = next_failure(&states).generation;

        let renamed = core
            .update_account_profile(
                serde_json::from_value(json!({
                    "accountId": account.id,
                    "profile": {"displayName": "Renamed", "color": "#2563eb"},
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(renamed.revision, account.revision);
        assert_eq!(core.sync_states()[0].generation, generation);

        let mut value = config(json!({"kind":"direct"}));
        value["imap"]["host"] = json!("127.0.0.1");
        value["imap"]["port"] = json!(closed_port());
        value["authentication"]["password"] = json!({"action":"keep"});
        let moved = core
            .update_account_connection(connection_update(&account.id, account.revision, value))
            .unwrap();
        assert_eq!(moved.display_name, "Renamed");
        assert_eq!(core.sync_states()[0].generation, generation + 1);
    }

    #[test]
    fn workers_resume_for_existing_accounts_on_open() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = TempDir::new().unwrap();
        let account = {
            let (core, states) = open(&directory, &runtime);
            let account = create(&core, closed_port());
            next_failure(&states);
            account
        };
        let (_core, states) = open(&directory, &runtime);
        assert_eq!(next_failure(&states).account_id, account.id);
    }
}
