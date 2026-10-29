//! The application service every host (desktop, later web and MCP) talks to.
//!
//! It owns the account store and the background sync workers, so account
//! changes and sync stay consistent no matter which host made the change.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::runtime::Handle;

use crate::accounts::AccountService;
use crate::accounts::outbox::SendPlan;
use crate::accounts::types::{
    Account, AccountRemoval, CreateAccountRequest, RemoveAccountRequest, UpdateConnectionRequest,
    UpdateProfileRequest,
};
use crate::drafts::attachments::{AddAttachmentRequest, AttachmentPolicy, RemoveAttachmentRequest};
use crate::drafts::types::{
    CreateDraftFromRequest, CreateDraftRequest, Draft, DraftSummary, UpdateDraftRequest,
};
use crate::error::{AppError, ErrorCode};
use crate::events::{CoreEvent, Events};
use crate::mail::{EmailDetail, EmailListQuery, EmailPage, Folder};
use crate::secrets::SecretStore;
use crate::send::types::{
    Caller, DraftChange, DraftChanged, OutboxItem, QueueSendRequest, ResolveUnknownRequest,
    UnknownResolution,
};
use crate::send::{OutboxManager, SendConfig};
use crate::storage::outbox::Changed;
use crate::sync::bodies::BodyManager;
use crate::sync::manager::{AccountSyncState, SyncManager};

mod attachments;
mod avatars;
mod bodies;
#[cfg(any(test, feature = "body-webview-test"))]
mod body_fixture;
mod flags;
mod verification;
pub use crate::sync::cadence::Activity;
pub use attachments::{AttachmentFile, AttachmentInfo};
pub use avatars::{AvatarImage, AvatarKind};
pub use bodies::{BodyBinding, BodyDocument, BodyLinks, RemoteImageStatus};
pub use flags::{FlagChange, MessageFlag};
pub use verification::{
    AccountConnectionVerification, CancelConnectionVerification, CancelDraftVerification,
    ConnectionVerificationCode, ConnectionVerificationStage, ConnectionVerificationStatus,
    DraftConnectionVerification, ProtocolVerification, VerifyAccountConnection,
    VerifyDraftConnection,
};

/// Cheap to clone; every clone shares the same store and workers. Methods
/// block on the store for a short transaction and must not be called from
/// inside an async task without `spawn_blocking`.
#[derive(Clone)]
pub struct MailxCore {
    // Declared first so it runs before the other fields are dropped.
    _closing: Arc<Closing>,
    service: Arc<Mutex<AccountService>>,
    sync: Arc<SyncManager>,
    outbox: Arc<OutboxManager>,
    bodies: Arc<BodyManager>,
    avatars: Arc<avatars::AvatarCache>,
    images: Arc<crate::storage::image_cache::ImageCache>,
    events: Events,
    verification: Arc<verification::VerificationRegistry>,
    draft_verification: Arc<verification::VerificationRegistry>,
}

/// Shared by every clone of a `MailxCore`. When the last clone goes, it stops
/// the background tasks, waits for them to end, then waits for blocking work
/// they handed off to let go of the store, so the data directory lock is free
/// once the core is dropped rather than whenever a straggling task finishes.
/// Every hand-off takes its store reference before spawning, so once the tasks
/// have ended no new reference can appear. Both waits are bounded.
struct Closing {
    service: std::sync::Weak<Mutex<AccountService>>,
    sync: Arc<SyncManager>,
    outbox: Arc<OutboxManager>,
    bodies: Arc<BodyManager>,
}

impl Drop for Closing {
    fn drop(&mut self) {
        let mut stopped = self.sync.shutdown();
        stopped.extend(self.outbox.shutdown());
        stopped.extend(self.bodies.shutdown());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let pause = || std::thread::sleep(std::time::Duration::from_millis(5));
        while !stopped.iter().all(|task| task.is_finished()) && std::time::Instant::now() < deadline
        {
            pause();
        }
        // The last `MailxCore` still holds one reference of its own.
        while self.service.strong_count() > 1 && std::time::Instant::now() < deadline {
            pause();
        }
    }
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
        Self::open_with(path, secrets, runtime, events, SendConfig::default())
    }

    /// `open` with the send queue's clock, SMTP budgets and trust store
    /// replaced; tests use it with local fakes.
    pub(crate) fn open_with(
        path: &Path,
        secrets: Arc<dyn SecretStore>,
        runtime: Handle,
        events: Events,
        send: SendConfig,
    ) -> Result<Self, AppError> {
        // Create and lock the profile before touching its shared image cache.
        // On first launch the profile directory may not exist yet.
        let service = AccountService::open(path, secrets)?;
        let images = Arc::new(crate::storage::image_cache::ImageCache::open(path)?);
        let accounts = service.list()?;
        let service = Arc::new(Mutex::new(service));
        let bodies = Arc::new(BodyManager::new(&service, runtime.clone()));
        let sync = Arc::new(SyncManager::new(
            service.clone(),
            runtime.clone(),
            events.clone(),
        ));
        for account in &accounts {
            sync.restart(&account.id);
        }
        let outbox = Arc::new(OutboxManager::new(&service, runtime, events.clone(), send));
        // Items interrupted by the last shutdown are settled before any worker
        // looks at the queue: no body start mark means nothing reached the
        // server, so the item is queued again; with the mark it is `unknown`.
        let recovered = service
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .recover_outbox(outbox.now())?;
        outbox.shared.announce(recovered);
        // Blank drafts nobody can be editing any more are crash leftovers.
        service
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .purge_blank_drafts()?;
        for account in &accounts {
            outbox.start(&account.id);
        }
        Ok(Self {
            _closing: Arc::new(Closing {
                service: Arc::downgrade(&service),
                sync: sync.clone(),
                outbox: outbox.clone(),
                bodies: bodies.clone(),
            }),
            service,
            sync,
            outbox,
            bodies,
            avatars: Arc::default(),
            images,
            events,
            verification: Arc::default(),
            draft_verification: Arc::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn service_for_tests(&self) -> Arc<Mutex<AccountService>> {
        self.service.clone()
    }

    #[cfg(test)]
    pub(crate) fn outbox_mime_for_tests(&self, id: &str) -> Vec<u8> {
        self.accounts().unwrap().outbox_mime(id).unwrap()
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
        self.outbox.start(&account.id);
        Ok(account)
    }

    /// Removes only local account data. The sending check and deletion share
    /// the service lock, so a queue worker cannot claim a message between them.
    pub fn remove_account(
        &self,
        request: RemoveAccountRequest,
    ) -> Result<AccountRemoval, AppError> {
        let removed = {
            let mut service = self.accounts()?;
            let removed = service.remove(request)?;
            // All body work takes these locks in service -> flights order.
            // The deletion has committed: retiring work must not report a
            // cleanup failure as though the account were still present.
            self.bodies.retire(&removed.account_id);
            removed
        };
        self.sync.retire(&removed.account_id);
        self.outbox.retire(&removed.account_id);
        self.verification.invalidate(&removed.account_id);
        self.events
            .emit(&CoreEvent::AccountRemoved(removed.clone()));
        for draft_id in &removed.draft_ids {
            self.events.emit(&CoreEvent::DraftChanged(DraftChanged {
                draft_id: draft_id.clone(),
                account_id: removed.account_id.clone(),
                change: DraftChange::Deleted,
            }));
        }
        Ok(removed)
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
        // Queued mail follows the new route and credentials on its next
        // attempt; a changed address fails the unsent items.
        self.outbox.wake(&account.id);
        self.verification.invalidate(&account.id);
        body_restart?;
        Ok(account)
    }

    /// Updates an account's profile. Sync and body downloads keep running.
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
        let draft = self.accounts()?.create_draft(request, default_account_id)?;
        self.announce_draft(&draft, DraftChange::Created);
        Ok(draft)
    }

    fn announce_draft(&self, draft: &Draft, change: DraftChange) {
        self.events.emit(&CoreEvent::DraftChanged(DraftChanged {
            draft_id: draft.id.clone(),
            account_id: draft.account_id.clone(),
            change,
        }));
    }

    /// Creates a reply, reply-all or forward draft from a message whose body
    /// is cached. Never downloads: an uncached body is `BodyNotCached`, and the
    /// caller fetches it with `fetch_body` first.
    pub fn create_draft_from(&self, request: CreateDraftFromRequest) -> Result<Draft, AppError> {
        self.create_draft_from_localized(request, crate::SupportedLocale::ZhCn)
    }

    /// The host supplies language only for newly generated quote text. Replayed
    /// mutation ids return the original draft without rewriting its contents.
    pub fn create_draft_from_localized(
        &self,
        request: CreateDraftFromRequest,
        locale: crate::SupportedLocale,
    ) -> Result<Draft, AppError> {
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
        // A forward carries the original files over (sending.md section 16.8).
        // Only this first creation prefills; a replay returned above adds nothing.
        let forwarded = match request.kind {
            crate::DraftKind::Forward => {
                let blobs = self.accounts()?.body_blobs();
                crate::drafts::attachments::forwarded_files(&raw, &blobs)?
            }
            _ => Vec::new(),
        };
        let draft = self.accounts()?.create_draft_from_source(
            &request,
            &crate::accounts::SourceMessage {
                detail: &detail,
                account_address: &account_address,
                headers: &headers,
                forwarded: &forwarded,
            },
            crate::accounts::local_offset(),
            locale,
        )?;
        self.announce_draft(&draft, DraftChange::Created);
        Ok(draft)
    }

    pub fn get_draft(&self, id: &str) -> Result<Draft, AppError> {
        self.accounts()?.get_draft(id)
    }

    /// Newest first; optionally only one account's drafts.
    pub fn list_drafts(&self, account_id: Option<&str>) -> Result<Vec<DraftSummary>, AppError> {
        self.accounts()?.list_drafts(account_id)
    }

    pub fn update_draft(&self, request: UpdateDraftRequest) -> Result<Draft, AppError> {
        let draft = self.accounts()?.update_draft(request)?;
        self.announce_draft(&draft, DraftChange::Updated);
        Ok(draft)
    }

    /// Attaches `bytes` (read by the host) to a draft as `request.file_name`.
    pub fn add_attachment(
        &self,
        request: AddAttachmentRequest,
        bytes: &[u8],
    ) -> Result<Draft, AppError> {
        let draft = self.accounts()?.add_attachment(request, bytes)?;
        self.announce_draft(&draft, DraftChange::Updated);
        Ok(draft)
    }

    pub fn remove_attachment(&self, request: RemoveAttachmentRequest) -> Result<Draft, AppError> {
        let draft = self.accounts()?.remove_attachment(request)?;
        self.announce_draft(&draft, DraftChange::Updated);
        Ok(draft)
    }

    pub fn attachment_policy(&self, account_id: &str) -> Result<AttachmentPolicy, AppError> {
        self.accounts()?.attachment_policy(account_id)
    }

    pub fn delete_draft(&self, id: &str) -> Result<(), AppError> {
        let draft = self.accounts()?.get_draft(id)?;
        self.accounts()?.delete_draft(id)?;
        self.announce_draft(&draft, DraftChange::Deleted);
        Ok(())
    }

    /// Queues a draft for sending: the MIME is built now from the draft and the
    /// account's current sender, the draft is locked, and the account's worker
    /// sends it once `undo_send_seconds` (from `compose.undoSendSeconds`; one
    /// of 0, 5, 10, 20, 30) have passed. Only the interactive composer may
    /// call this; any other `caller` gets `Forbidden`.
    pub fn queue_send(
        &self,
        caller: Caller,
        request: QueueSendRequest,
        undo_send_seconds: u32,
    ) -> Result<OutboxItem, AppError> {
        if caller != Caller::Composer {
            return Err(AppError::new(ErrorCode::Forbidden));
        }
        // The store lock is held to read the draft and again to commit; reading
        // attachment blobs and building the MIME happen in between, without it.
        let plan = self.accounts()?.prepare_send(&request, undo_send_seconds)?;
        let (item, locked) = match plan {
            SendPlan::Replay(item) => (*item, None),
            SendPlan::Build(build) => {
                let frozen = build.freeze()?;
                self.accounts()?.commit_send(
                    &request,
                    undo_send_seconds,
                    &build,
                    frozen,
                    self.outbox.now(),
                )?
            }
        };
        if let Some(locked) = locked {
            self.outbox.shared.announce(vec![Changed {
                item: item.clone(),
                draft: Some(locked),
            }]);
            self.outbox.wake(&item.account_id);
        }
        Ok(item)
    }

    /// Queue items, newest first. Without `include_all` only those that need
    /// attention or time: queued, sending, failed and unsettled unknown.
    pub fn list_outbox(
        &self,
        account_id: Option<&str>,
        include_all: bool,
    ) -> Result<Vec<OutboxItem>, AppError> {
        self.accounts()?.list_outbox(account_id, include_all)
    }

    pub fn get_outbox(&self, id: &str) -> Result<OutboxItem, AppError> {
        self.accounts()?.get_outbox(id)
    }

    /// Undo send. Succeeds only while the item is still `queued`; once the
    /// worker has started it the answer is `AlreadySending`.
    pub fn cancel_outbox(&self, id: &str) -> Result<OutboxItem, AppError> {
        let changed = self.accounts()?.cancel_outbox(id, self.outbox.now())?;
        let item = changed.item.clone();
        self.outbox.shared.announce(vec![changed]);
        self.outbox.wake(&item.account_id);
        Ok(item)
    }

    /// Skips the backoff wait of a `queued` item.
    pub fn retry_outbox_now(&self, id: &str) -> Result<OutboxItem, AppError> {
        let item = self.accounts()?.retry_outbox_now(id, self.outbox.now())?;
        self.outbox.wake(&item.account_id);
        Ok(item)
    }

    /// Settles an `unknown` item (interactive callers only: the composer or the
    /// main window): `MarkSent` accepts it; `Resend` queues a new
    /// attempt with the same Message-ID and bytes and returns that new item.
    pub fn resolve_unknown(
        &self,
        caller: Caller,
        request: ResolveUnknownRequest,
    ) -> Result<OutboxItem, AppError> {
        if !matches!(caller, Caller::Composer | Caller::MainWindow) {
            return Err(AppError::new(ErrorCode::Forbidden));
        }
        let (changed, old) = {
            let mut service = self.accounts()?;
            match service.resolve_unknown(&request, self.outbox.now())? {
                None => return service.resolved_item(&request),
                Some(changed) => {
                    let old = service.get_outbox(&request.id)?;
                    (changed, old)
                }
            }
        };
        let item = changed.item.clone();
        let mut announced = Vec::new();
        if request.resolution == UnknownResolution::Resend {
            announced.push(Changed {
                item: old,
                draft: None,
            });
        }
        announced.push(changed);
        self.outbox.shared.announce(announced);
        self.outbox.wake(&item.account_id);
        Ok(item)
    }

    /// Prevents new send attempts from being claimed. Already claimed SMTP
    /// transactions continue through their result commit; hosts must wait for
    /// `shutdown_idle` before exiting, or call `cancel_shutdown` to keep running.
    pub fn begin_shutdown(&self) {
        self.outbox.begin_shutdown();
    }

    pub fn shutdown_idle(&self) -> bool {
        self.outbox.shutdown_idle()
    }

    pub fn cancel_shutdown(&self) {
        self.outbox.cancel_shutdown();
    }

    /// Tells the sync workers how the app is being used. While it is in front
    /// they check often; in the background they check less and less often when
    /// nothing arrives; while the computer sleeps they pause. Coming to the
    /// front starts a round at once. Hosts without such a notion never call this
    /// and stay on the in-front rhythm.
    pub fn set_activity(&self, activity: Activity) {
        self.sync.set_activity(activity);
    }

    /// What the sync workers were last told about how the app is used.
    pub fn activity(&self) -> Activity {
        self.sync.activity()
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
    fn account_removal_retires_sync_and_announces_every_deleted_draft() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = TempDir::new().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        let core = MailxCore::open(
            directory.path(),
            Arc::new(MemorySecrets::default()),
            runtime.handle().clone(),
            Arc::new(move |event: &CoreEvent| sink.lock().unwrap().push(event.clone())),
        )
        .unwrap();
        let account = create(&core, closed_port());
        let draft = core
            .create_draft(
                CreateDraftRequest {
                    account_id: Some(account.id.clone()),
                    to: vec![],
                    cc: vec![],
                    bcc: vec![],
                    subject: "local draft".into(),
                    body_html: "<p>text</p>".into(),
                    client_mutation_id: Uuid::new_v4().to_string(),
                },
                "",
            )
            .unwrap();
        assert_eq!(
            core.remove_account(RemoveAccountRequest {
                account_id: account.id.clone(),
                expected_revision: account.revision + 1,
            })
            .unwrap_err()
            .code,
            ErrorCode::Conflict
        );
        assert_eq!(core.get_draft(&draft.id).unwrap().id, draft.id);
        assert!(
            core.sync_states()
                .iter()
                .any(|state| state.account_id == account.id)
        );
        assert!(
            !captured
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, CoreEvent::AccountRemoved(_)))
        );
        let removed = core
            .remove_account(RemoveAccountRequest {
                account_id: account.id.clone(),
                expected_revision: account.revision,
            })
            .unwrap();
        assert_eq!(removed.draft_ids, vec![draft.id.clone()]);
        assert!(core.list_accounts().unwrap().is_empty());
        assert!(core.sync_states().is_empty());
        assert_eq!(
            core.get_draft(&draft.id).unwrap_err().code,
            ErrorCode::NotFound
        );
        // An update/create may have committed before removal but invoke these
        // starts afterwards; neither may restore the removed runtime state.
        core.sync.restart(&account.id);
        core.outbox.wake(&account.id);
        assert!(core.sync_states().is_empty());
        let events = captured.lock().unwrap();
        let removal_index = events
            .iter()
            .position(
                |event| matches!(event, CoreEvent::AccountRemoved(value) if value == &removed),
            )
            .unwrap();
        let deleted_index = events
            .iter()
            .position(|event| {
                matches!(event, CoreEvent::DraftChanged(value)
                if value.draft_id == draft.id && value.change == DraftChange::Deleted)
            })
            .unwrap();
        assert!(removal_index < deleted_index);
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
                    "profile": {"displayName": "Renamed", "color": "#2563eb", "useProviderIcon": false},
                }))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(renamed.revision, account.revision);
        assert!(!renamed.use_provider_icon);
        assert_eq!(core.sync_states()[0].generation, generation);

        let mut value = config(json!({"kind":"direct"}));
        value["imap"]["host"] = json!("127.0.0.1");
        value["imap"]["port"] = json!(closed_port());
        value["authentication"]["password"] = json!({"action":"keep"});
        let moved = core
            .update_account_connection(connection_update(&account.id, account.revision, value))
            .unwrap();
        assert_eq!(moved.display_name, "Renamed");
        assert!(!moved.use_provider_icon);
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

    #[test]
    fn first_launch_creates_a_new_profile_before_opening_the_image_cache() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = TempDir::new().unwrap();
        let profile = directory.path().join("new-profile");
        assert!(!profile.exists());
        let core = MailxCore::open(
            &profile,
            Arc::new(MemorySecrets::default()),
            runtime.handle().clone(),
            Arc::new(|_: &crate::events::CoreEvent| {}),
        )
        .unwrap();
        assert!(core.list_accounts().unwrap().is_empty());
        assert!(profile.join("image-cache").is_dir());
        assert!(profile.join("mailx.sqlite3").is_file());
    }

    #[test]
    fn dropping_the_core_frees_the_directory_for_an_immediate_reopen() {
        // Background tasks used to keep the store, and its lock, alive for a
        // moment after the core was dropped.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = TempDir::new().unwrap();
        {
            let (core, states) = open(&directory, &runtime);
            create(&core, closed_port());
            next_failure(&states);
        }
        for _ in 0..20 {
            let (core, _states) = open(&directory, &runtime);
            let _clone = core.clone();
        }
    }
}
