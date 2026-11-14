//! Shared, bounded verification-code reads and copy admission for trusted hosts.
//! Recognition is local and cached only in memory. Disposable IMAP reads never
//! alter body/read state, and dropping the caller cancels the network future.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use ts_rs::TS;
use uuid::Uuid;

use super::{DELETE_UNDO_WINDOW, MailxCore};
use crate::{
    AppError, EmailDeletion, ErrorCode,
    accounts::{AccountService, sync_plan::SyncPlan},
    imap,
    mail::verification_code::{self, Candidate, Detection, MAX_EML_BYTES},
    storage::bodies::{BlobStore, BodyIdentity, StoredBlob},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_AGE: chrono::Duration = chrono::Duration::minutes(10);
const CACHE_CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum VerificationCodeState {
    Unchecked,
    Ready,
    NotFound,
    Ambiguous,
    Unavailable,
}

/// Only trusted application UI receives these values. Debug never prints the
/// code or the recognition capability; explicit serialization is intentional.
#[derive(Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCodeStatus {
    pub email_id: String,
    pub state: VerificationCodeState,
    pub code: Option<String>,
    pub recognition_id: Option<String>,
}

impl fmt::Debug for VerificationCodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerificationCodeStatus")
            .field("email_id", &self.email_id)
            .field("state", &self.state)
            .field("code", &self.code.as_ref().map(|_| "[redacted]"))
            .field(
                "recognition_id",
                &self.recognition_id.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

impl VerificationCodeStatus {
    fn empty(email_id: String, state: VerificationCodeState) -> Self {
        Self {
            email_id,
            state,
            code: None,
            recognition_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum VerificationCodeCleanup {
    Disabled,
    Queued,
    /// Trash is unavailable, or this mail is already in Trash. Never expunge.
    Unavailable,
    /// The code was copied, but scheduling the cleanup failed.
    Failed,
}

/// A returned value always means the clipboard callback succeeded. Cleanup
/// has its own outcome, so a database failure cannot misreport a failed copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCodeCopyOutcome {
    pub email_id: String,
    pub account_id: String,
    pub cleanup: VerificationCodeCleanup,
    pub deletion: Option<EmailDeletion>,
}

pub(super) struct NotificationCodes {
    gate: Semaphore,
    cache: Mutex<VecDeque<Recognized>>,
    /// Weak gates vanish with their last caller; waiting callers share the
    /// completed cache result rather than starting duplicate MIME downloads.
    serial: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
    #[cfg(test)]
    connector: Mutex<Option<TestConnector>>,
}

#[cfg(test)]
type TestConnector = tokio::sync::mpsc::UnboundedSender<
    tokio::sync::oneshot::Sender<Result<imap::Session, imap::ImapError>>,
>;

impl Default for NotificationCodes {
    fn default() -> Self {
        Self {
            gate: Semaphore::new(2),
            cache: Mutex::default(),
            serial: Mutex::default(),
            #[cfg(test)]
            connector: Mutex::new(None),
        }
    }
}

#[derive(Clone)]
struct Recognized {
    identity: BodyIdentity,
    raw_hash: String,
    had_cached_body: bool,
    state: VerificationCodeState,
    candidate: Option<Candidate>,
    recognition_id: Option<String>,
}

impl Recognized {
    fn current(&self, identity: &BodyIdentity, stamp: Option<&StoredBlob>) -> bool {
        self.identity.same_message(identity)
            && match stamp {
                Some(raw) => self.raw_hash == raw.sha256,
                None => !self.had_cached_body,
            }
    }

    fn status(&self) -> VerificationCodeStatus {
        VerificationCodeStatus {
            email_id: self.identity.message_id.clone(),
            state: self.state,
            code: self
                .candidate
                .as_ref()
                .map(|candidate| candidate.code.clone()),
            recognition_id: self.recognition_id.clone(),
        }
    }
}

impl NotificationCodes {
    /// Account removal already holds the service lock, so no completed read
    /// can insert another result for that account after its data disappears.
    pub(super) fn retire(&self, account_id: &str) {
        self.cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|entry| entry.identity.account_id != account_id);
    }

    fn serial(&self, email_id: &str) -> Result<Arc<AsyncMutex<()>>, AppError> {
        let mut serial = self.serial.lock().map_err(|_| internal())?;
        serial.retain(|_, value| value.strong_count() > 0);
        if let Some(gate) = serial.get(email_id).and_then(Weak::upgrade) {
            return Ok(gate);
        }
        let gate = Arc::new(AsyncMutex::new(()));
        serial.insert(email_id.to_owned(), Arc::downgrade(&gate));
        Ok(gate)
    }

    /// Caller holds the service lock: all cache admission uses service -> cache.
    fn cached(
        &self,
        identity: &BodyIdentity,
        stamp: Option<&StoredBlob>,
    ) -> Result<Option<Recognized>, AppError> {
        let mut cache = self.cache.lock().map_err(|_| internal())?;
        let Some(index) = cache
            .iter()
            .position(|entry| entry.identity.message_id == identity.message_id)
        else {
            return Ok(None);
        };
        let entry = cache.remove(index).expect("cache index exists");
        if !entry.current(identity, stamp) {
            return Ok(None);
        }
        cache.push_back(entry.clone());
        Ok(Some(entry))
    }

    fn insert(&self, entry: Recognized) -> Result<VerificationCodeStatus, AppError> {
        let mut cache = self.cache.lock().map_err(|_| internal())?;
        cache.retain(|current| current.identity.message_id != entry.identity.message_id);
        while cache.len() >= CACHE_CAPACITY {
            cache.pop_front();
        }
        let status = entry.status();
        cache.push_back(entry);
        Ok(status)
    }
}

enum Input {
    Cached(BlobStore, StoredBlob),
    Network(SyncPlan),
}

enum Prepared {
    Done(VerificationCodeStatus),
    Read(Box<(BodyIdentity, Input)>),
}

impl MailxCore {
    /// Status for a visible server message, independent of notification
    /// settings, unread flags, folder role and age. Cached complete MIME may
    /// be checked with any subject; missing bodies are fetched only when the
    /// caller permits it and the subject is a likely verification message.
    /// At most two reads run, each with a four-second total caller budget.
    pub async fn verification_code_status(
        &self,
        email_id: String,
        allow_fetch: bool,
    ) -> Result<VerificationCodeStatus, AppError> {
        let fallback = email_id.clone();
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            self.verification_code_inner(email_id, allow_fetch),
        )
        .await
        .unwrap_or_else(|_| {
            Ok(VerificationCodeStatus::empty(
                fallback,
                VerificationCodeState::Unavailable,
            ))
        })
    }

    /// An inexpensive local status lookup, for code already off the async
    /// runtime. No network or MIME parsing; stale entries are discarded.
    pub fn cached_verification_code_status(
        &self,
        email_id: &str,
    ) -> Result<VerificationCodeStatus, AppError> {
        let service = self.accounts()?;
        let Some((_subject, identity)) = visible_identity(&service, email_id)? else {
            return Ok(VerificationCodeStatus::empty(
                email_id.to_owned(),
                VerificationCodeState::Unavailable,
            ));
        };
        let stamp = service.body_stamp(&identity)?.map(|(raw, _)| raw);
        Ok(self
            .notification_codes
            .cached(&identity, stamp.as_ref())?
            .map(|entry| entry.status())
            .unwrap_or_else(|| {
                VerificationCodeStatus::empty(email_id.to_owned(), VerificationCodeState::Unchecked)
            }))
    }

    /// Resolve the application's recognition capability and invoke the host's
    /// clipboard write, then optionally queue a reversible move to Trash.
    ///
    /// The caller supplies the persisted preference, never a code string. The
    /// callback must not re-enter core: the service lock keeps source changes,
    /// removal and deletion from changing the target during this operation.
    /// Clipboard failure queues nothing. Cleanup failure preserves copy success.
    pub fn copy_verification_code(
        &self,
        email_id: &str,
        recognition_id: &str,
        trash_after_copy: bool,
        write: impl FnOnce(&str) -> Result<(), AppError>,
    ) -> Result<VerificationCodeCopyOutcome, AppError> {
        self.copy_verification_code_with_policy(email_id, recognition_id, |code| {
            write(code)?;
            Ok(trash_after_copy)
        })
    }

    /// The host can sample its latest privacy/cleanup policy after service
    /// admission, perform the clipboard write, and return whether to clean up.
    /// The callback must not re-enter core; it runs under the same service lock
    /// as identity validation and deletion scheduling. Returning an error means
    /// no cleanup is scheduled, and the host must not have reported copy success.
    pub fn copy_verification_code_with_policy(
        &self,
        email_id: &str,
        recognition_id: &str,
        write_and_policy: impl FnOnce(&str) -> Result<bool, AppError>,
    ) -> Result<VerificationCodeCopyOutcome, AppError> {
        if Uuid::parse_str(recognition_id).is_err() {
            return Err(AppError::validation("recognitionId"));
        }
        let mut service = self.accounts()?;
        let Some((_subject, identity)) = visible_identity(&service, email_id)? else {
            return Err(not_found());
        };
        let stamp = service.body_stamp(&identity)?.map(|(raw, _)| raw);
        let entry = self
            .notification_codes
            .cached(&identity, stamp.as_ref())?
            .ok_or_else(conflict)?;
        if entry.recognition_id.as_deref() != Some(recognition_id) {
            return Err(conflict());
        }
        let candidate = entry.candidate.ok_or_else(conflict)?;
        let trash_after_copy = write_and_policy(&candidate.code)?;
        let mut result = VerificationCodeCopyOutcome {
            email_id: email_id.to_owned(),
            account_id: identity.account_id,
            cleanup: VerificationCodeCleanup::Disabled,
            deletion: None,
        };
        let mut wake = Vec::new();
        if trash_after_copy {
            match service.schedule_deletion_with(
                &[email_id.to_owned()],
                false,
                false,
                Utc::now(),
                DELETE_UNDO_WINDOW,
            ) {
                Ok(scheduled) => {
                    wake = scheduled.account_ids;
                    result.cleanup = VerificationCodeCleanup::Queued;
                    result.deletion = Some(EmailDeletion {
                        batch_id: scheduled.batch_id,
                        undo_until: scheduled
                            .due_at
                            .to_rfc3339_opts(SecondsFormat::Millis, true),
                        permanent: false,
                        skipped_ids: scheduled.skipped_ids,
                    });
                }
                Err(error) if error.code == ErrorCode::ConfirmationRequired => {
                    result.cleanup = VerificationCodeCleanup::Unavailable
                }
                Err(_) => result.cleanup = VerificationCodeCleanup::Failed,
            }
        }
        drop(service);
        for account in wake {
            self.sync.changes_waiting(&account);
        }
        Ok(result)
    }

    /// The notification host still applies privacy settings and admits IDs
    /// only from an actual NewMail event. Its stricter recent/unread/INBOX
    /// eligibility is separate from the shared UI recognition service.
    pub async fn notification_verification_code(
        &self,
        email_id: String,
    ) -> Result<Option<Candidate>, AppError> {
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            let service = self.service.clone();
            let id = email_id.clone();
            let admitted = tokio::task::spawn_blocking(move || {
                let service = service.lock().map_err(|_| internal())?;
                eligible(&service, &id, true)
            })
            .await
            .map_err(|_| internal())??;
            if admitted.is_none() {
                return Ok(None);
            }
            let status = self
                .verification_code_status(email_id.clone(), true)
                .await?;
            if status.state != VerificationCodeState::Ready {
                return Ok(None);
            }
            let service = self.service.clone();
            let codes = self.notification_codes.clone();
            tokio::task::spawn_blocking(move || {
                let service = service.lock().map_err(|_| internal())?;
                let Some(identity) = eligible(&service, &email_id, true)? else {
                    return Ok(None);
                };
                let stamp = service.body_stamp(&identity)?.map(|(raw, _)| raw);
                Ok(codes
                    .cached(&identity, stamp.as_ref())?
                    .and_then(|entry| entry.candidate))
            })
            .await
            .map_err(|_| internal())?
        })
        .await
        .unwrap_or(Ok(None))
    }

    /// Run on a blocking thread. Notification actions still have their own age
    /// and inbox policy; UI copy uses the capability and current identity.
    pub fn notification_code_is_current(
        &self,
        account_id: &str,
        email_id: &str,
    ) -> Result<bool, AppError> {
        let service = self.accounts()?;
        Ok(eligible(&service, email_id, false)?
            .is_some_and(|identity| identity.account_id == account_id))
    }

    async fn verification_code_inner(
        &self,
        email_id: String,
        allow_fetch: bool,
    ) -> Result<VerificationCodeStatus, AppError> {
        let serial = self.notification_codes.serial(&email_id)?;
        let _serial = serial.lock().await;
        // Already recognized mail must remain instant while other messages are
        // waiting on slow servers. Only actual reading/parsing takes the gate.
        let core = self.clone();
        let id = email_id.clone();
        let cached = tokio::task::spawn_blocking(move || core.cached_verification_code_status(&id))
            .await
            .map_err(|_| internal())??;
        if cached.state != VerificationCodeState::Unchecked {
            return Ok(cached);
        }
        let _permit = self
            .notification_codes
            .gate
            .acquire()
            .await
            .map_err(|_| internal())?;
        let service = self.service.clone();
        let codes = self.notification_codes.clone();
        let id = email_id.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let service = service.lock().map_err(|_| internal())?;
            let empty = |state| Prepared::Done(VerificationCodeStatus::empty(id.clone(), state));
            let Some((subject, identity)) = visible_identity(&service, &id)? else {
                return Ok(empty(VerificationCodeState::Unavailable));
            };
            let stamp = service.body_stamp(&identity)?.map(|(raw, _)| raw);
            if let Some(entry) = codes.cached(&identity, stamp.as_ref())? {
                return Ok(Prepared::Done(entry.status()));
            }
            let input = if let Some(raw) = stamp {
                if raw.size > MAX_EML_BYTES as u64 {
                    return Ok(empty(VerificationCodeState::Unavailable));
                }
                Input::Cached(service.body_blobs(), raw)
            } else {
                if !allow_fetch || !verification_code::is_candidate_subject(&subject) {
                    return Ok(empty(VerificationCodeState::Unchecked));
                }
                if identity
                    .size
                    .is_some_and(|size| size > MAX_EML_BYTES as u64)
                {
                    return Ok(empty(VerificationCodeState::Unavailable));
                }
                let Ok(plan) = service.sync_plan(&identity.account_id) else {
                    return Ok(empty(VerificationCodeState::Unavailable));
                };
                Input::Network(plan)
            };
            Ok::<_, AppError>(Prepared::Read(Box::new((identity, input))))
        })
        .await
        .map_err(|_| internal())??;
        let (identity, input) = match prepared {
            Prepared::Done(status) => return Ok(status),
            Prepared::Read(read) => *read,
        };
        let had_cached_body = matches!(input, Input::Cached(..));
        let raw = match input {
            Input::Cached(blobs, raw) => {
                tokio::task::spawn_blocking(move || blobs.read_limited(&raw, MAX_EML_BYTES))
                    .await
                    .map_err(|_| internal())??
            }
            Input::Network(plan) => {
                let Some(raw) = self.notification_raw(&identity, &plan).await else {
                    return Ok(VerificationCodeStatus::empty(
                        email_id,
                        VerificationCodeState::Unavailable,
                    ));
                };
                raw
            }
        };
        let (raw_hash, detection) = tokio::task::spawn_blocking(move || {
            (
                format!("{:x}", Sha256::digest(&raw)),
                verification_code::detect_eml(&raw),
            )
        })
        .await
        .map_err(|_| internal())?;
        let (state, candidate) = match detection {
            Ok(Detection::Found { candidate }) => (VerificationCodeState::Ready, Some(candidate)),
            Ok(Detection::Ambiguous { .. }) => (VerificationCodeState::Ambiguous, None),
            Ok(Detection::NotFound) => (VerificationCodeState::NotFound, None),
            Err(_) => (VerificationCodeState::Unavailable, None),
        };
        let service = self.service.clone();
        let codes = self.notification_codes.clone();
        tokio::task::spawn_blocking(move || {
            let service = service.lock().map_err(|_| internal())?;
            let Some((_subject, current)) = visible_identity(&service, &email_id)? else {
                return Ok(VerificationCodeStatus::empty(
                    email_id,
                    VerificationCodeState::Unavailable,
                ));
            };
            let stamp = service.body_stamp(&current)?.map(|(raw, _)| raw);
            let entry = Recognized {
                identity,
                raw_hash,
                had_cached_body,
                state,
                recognition_id: candidate.as_ref().map(|_| Uuid::new_v4().to_string()),
                candidate,
            };
            if !entry.current(&current, stamp.as_ref()) {
                return Ok(VerificationCodeStatus::empty(
                    email_id,
                    VerificationCodeState::Unavailable,
                ));
            }
            codes.insert(entry)
        })
        .await
        .map_err(|_| internal())?
    }

    async fn notification_raw(&self, identity: &BodyIdentity, plan: &SyncPlan) -> Option<Vec<u8>> {
        let mut session = self.notification_connection(plan).await.ok()?;
        if session
            .examine(&identity.remote_name)
            .await
            .ok()?
            .uid_validity
            != identity.uid_validity
        {
            return None;
        }
        session
            .fetch_message_limited(identity.uid, identity.size, MAX_EML_BYTES)
            .await
            .ok()
            .flatten()
    }

    async fn notification_connection(
        &self,
        plan: &SyncPlan,
    ) -> Result<imap::Session, imap::ImapError> {
        #[cfg(test)]
        {
            let connector = self.notification_codes.connector.lock().unwrap().clone();
            if let Some(connector) = connector {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                connector
                    .send(sender)
                    .map_err(|_| imap::ImapError::Rejected)?;
                return receiver.await.map_err(|_| imap::ImapError::Rejected)?;
            }
        }
        imap::open_body_session(&plan.route, &plan.login).await
    }
}

fn internal() -> AppError {
    AppError::new(ErrorCode::Internal)
}
fn not_found() -> AppError {
    AppError::new(ErrorCode::NotFound)
}
fn conflict() -> AppError {
    AppError::new(ErrorCode::Conflict)
}

fn visible_identity(
    service: &AccountService,
    email_id: &str,
) -> Result<Option<(String, BodyIdentity)>, AppError> {
    let Some(subject) = service.verification_code_header(email_id)? else {
        return Ok(None);
    };
    Ok(Some((subject, service.body_identity(email_id)?)))
}

fn eligible(
    service: &AccountService,
    email_id: &str,
    unread: bool,
) -> Result<Option<BodyIdentity>, AppError> {
    let Some((subject, received_at, is_read)) = service.notification_header(email_id)? else {
        return Ok(None);
    };
    if unread && (is_read || !verification_code::is_candidate_subject(&subject)) {
        return Ok(None);
    }
    let Ok(received) = DateTime::parse_from_rfc3339(&received_at) else {
        return Ok(None);
    };
    let age = Utc::now().signed_duration_since(received);
    if age < chrono::Duration::zero() || age > MAX_AGE {
        return Ok(None);
    }
    Ok(Some(service.body_identity(email_id)?))
}

#[cfg(test)]
mod tests;
