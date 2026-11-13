//! Bounded, disposable body reads for a trusted host's new-mail notifications.
//! No cache/state mutation and no detached network task: dropping the caller's
//! future closes the connection instead of leaving a normal body fetch running.

use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Semaphore;

use super::MailxCore;
use crate::{
    AppError, ErrorCode,
    accounts::{AccountService, sync_plan::SyncPlan},
    imap,
    mail::verification_code::{self, Candidate, Detection, MAX_EML_BYTES},
    storage::bodies::{BlobStore, BodyIdentity, StoredBlob},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_AGE: chrono::Duration = chrono::Duration::minutes(10);

pub(super) struct NotificationCodes {
    gate: Semaphore,
    #[cfg(test)]
    connector: std::sync::Mutex<Option<TestConnector>>,
}

#[cfg(test)]
type TestConnector = tokio::sync::mpsc::UnboundedSender<
    tokio::sync::oneshot::Sender<Result<imap::Session, imap::ImapError>>,
>;

impl Default for NotificationCodes {
    fn default() -> Self {
        Self {
            gate: Semaphore::new(2),
            #[cfg(test)]
            connector: std::sync::Mutex::new(None),
        }
    }
}

enum Input {
    Cached(BlobStore, StoredBlob),
    Network(SyncPlan),
}

impl MailxCore {
    /// Recognize one fresh arrival for a trusted notification host. The host
    /// first applies its notification/privacy settings and admits only IDs from
    /// an actual NewMail event; this method never scans historical mail.
    ///
    /// Candidate subjects only; at most two operations share a separate gate.
    /// The four-second budget includes waiting, credentials, transfer and local
    /// decoding. Any network, MIME, size or eligibility failure yields no code
    /// so the host can show its ordinary notification. No read flag or body
    /// cache state is changed. This is not an IPC or MCP endpoint.
    pub async fn notification_verification_code(
        &self,
        email_id: String,
    ) -> Result<Option<Candidate>, AppError> {
        tokio::time::timeout(REQUEST_TIMEOUT, self.notification_code_inner(email_id))
            .await
            .unwrap_or(Ok(None))
    }

    /// Cheap local revalidation immediately before a notification copy action.
    /// A user may already have opened the message, so read mail is permitted;
    /// removed, hidden, stale, non-inbox or old mail is refused. The caller must
    /// independently recheck notification/privacy preferences and its token.
    /// Run this synchronous query on a blocking thread, like `get_email`.
    pub fn notification_code_is_current(
        &self,
        account_id: &str,
        email_id: &str,
    ) -> Result<bool, AppError> {
        let service = self.accounts()?;
        Ok(eligible(&service, email_id, false)?
            .is_some_and(|identity| identity.account_id == account_id))
    }

    async fn notification_code_inner(
        &self,
        email_id: String,
    ) -> Result<Option<Candidate>, AppError> {
        let _permit = self
            .notification_codes
            .gate
            .acquire()
            .await
            .map_err(|_| internal())?;
        let service = self.service.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let service = service.lock().map_err(|_| internal())?;
            let Some(identity) = eligible(&service, &email_id, true)? else {
                return Ok(None);
            };
            let input = if let Some((raw, _)) = service.body_stamp(&identity)? {
                if raw.size > MAX_EML_BYTES as u64 {
                    return Ok(None);
                }
                Input::Cached(service.body_blobs(), raw)
            } else {
                if identity
                    .size
                    .is_some_and(|size| size > MAX_EML_BYTES as u64)
                {
                    return Ok(None);
                }
                let Ok(plan) = service.sync_plan(&identity.account_id) else {
                    return Ok(None);
                };
                Input::Network(plan)
            };
            Ok::<_, AppError>(Some((identity, input)))
        })
        .await
        .map_err(|_| internal())??;
        let Some((identity, input)) = prepared else {
            return Ok(None);
        };
        let raw = match input {
            Input::Cached(blobs, raw) => {
                tokio::task::spawn_blocking(move || blobs.read_limited(&raw, MAX_EML_BYTES))
                    .await
                    .map_err(|_| internal())??
            }
            Input::Network(plan) => {
                let Some(raw) = self.notification_raw(&identity, &plan).await else {
                    return Ok(None);
                };
                raw
            }
        };
        let detection = tokio::task::spawn_blocking(move || verification_code::detect_eml(&raw))
            .await
            .map_err(|_| internal())?;
        let Ok(Detection::Found { candidate }) = detection else {
            return Ok(None);
        };
        let service = self.service.clone();
        let current = tokio::task::spawn_blocking(move || {
            let service = service.lock().map_err(|_| internal())?;
            Ok::<_, AppError>(
                eligible(&service, &identity.message_id, true)?
                    .is_some_and(|current| current.same_message(&identity)),
            )
        })
        .await
        .map_err(|_| internal())??;
        Ok(current.then_some(candidate))
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
    let identity = service.body_identity(email_id)?;
    Ok(Some(identity))
}

#[cfg(test)]
mod tests;
