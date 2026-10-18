//! The application service every host (desktop, later web and MCP) talks to.
//!
//! It owns the account store and the background sync workers, so account
//! changes and sync stay consistent no matter which host made the change.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::runtime::Handle;

use crate::accounts::AccountService;
use crate::accounts::types::{Account, CreateAccountRequest, UpdateAccountRequest};
use crate::error::{AppError, ErrorCode};
use crate::mail::Folder;
use crate::secrets::SecretStore;
use crate::sync::manager::{AccountSyncState, SyncListener, SyncManager};

/// Cheap to clone; every clone shares the same store and workers. Methods
/// block on the store for a short transaction and must not be called from
/// inside an async task without `spawn_blocking`.
#[derive(Clone)]
pub struct MailxCore {
    service: Arc<Mutex<AccountService>>,
    sync: Arc<SyncManager>,
}

impl MailxCore {
    /// Opens the store in `path` and starts syncing every account on `runtime`.
    /// `listener` hears each sync state change.
    pub fn open(
        path: &Path,
        secrets: Arc<dyn SecretStore>,
        runtime: Handle,
        listener: SyncListener,
    ) -> Result<Self, AppError> {
        let service = AccountService::open(path, secrets)?;
        let accounts = service.list()?;
        let service = Arc::new(Mutex::new(service));
        let sync = Arc::new(SyncManager::new(service.clone(), runtime, listener));
        for account in &accounts {
            sync.restart(&account.id);
        }
        Ok(Self { service, sync })
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

    /// Updates an account and restarts its sync with the new settings, so no
    /// connection made under the old server, route or credentials survives.
    pub fn update_account(&self, request: UpdateAccountRequest) -> Result<Account, AppError> {
        let account = self.accounts()?.update(request)?;
        self.sync.restart(&account.id);
        Ok(account)
    }

    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>, AppError> {
        self.accounts()?.list_folders(account_id)
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
    use crate::accounts::sync_plan::tests::config;
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
        let listener: SyncListener = Arc::new(move |state| {
            let _ = sender.lock().unwrap().send(state.clone());
        });
        let core = MailxCore::open(
            directory.path(),
            Arc::new(MemorySecrets::default()),
            runtime.handle().clone(),
            listener,
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
        core.update_account(
            serde_json::from_value(
                json!({"accountId": account.id, "expectedRevision": account.revision, "config": value}),
            )
            .unwrap(),
        )
        .unwrap();

        let failure = next_failure(&states).failure.unwrap();
        assert_eq!(failure.code, SyncFailureCode::CredentialUntrusted);
        assert!(!failure.will_retry);
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
