//! Send queue wire contract. Nothing here carries message bodies or MIME.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Who is asking. Only the interactive composer may queue mail for sending
/// (sending.md section 3): a model or MCP principal never can, however it
/// calls. Settling an `unknown` item is also allowed from the main window,
/// the other interactive UI. The host decides the principal; the core never
/// infers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// The user's own composer window.
    Composer,
    /// The user's main window (outbox view); may settle `unknown` items but
    /// not queue mail.
    MainWindow,
    /// An MCP client.
    Mcp,
    /// A model acting on its own.
    Model,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum OutboxState {
    Queued,
    Sending,
    Accepted,
    Failed,
    Unknown,
    Canceled,
}

impl OutboxState {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "sending" => Self::Sending,
            "accepted" => Self::Accepted,
            "failed" => Self::Failed,
            "unknown" => Self::Unknown,
            "canceled" => Self::Canceled,
            _ => return None,
        })
    }
}

/// How the user settled an `unknown` item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum OutboxResolution {
    /// Declared delivered; the item became `accepted`.
    MarkedSent,
    /// Declared not delivered; a new item with the same Message-ID was queued.
    Resent,
}

impl OutboxResolution {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "markedSent" => Some(Self::MarkedSent),
            "resent" => Some(Self::Resent),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum UnknownResolution {
    MarkSent,
    Resend,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RejectedRecipientInfo {
    pub address: String,
    #[ts(type = "number")]
    pub code: u16,
    /// Sanitized server text.
    pub text: String,
}

/// Why the last attempt failed. Texts are sanitized and length-limited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OutboxError {
    /// A fixed code such as `connect`, `authRejected`, `recipientsRejected`,
    /// `rejectedAfterData`, `retryExhausted`, `accountChanged`, `unknown`.
    pub reason: String,
    #[ts(type = "number | null")]
    pub smtp_code: Option<u16>,
    pub text: Option<String>,
    pub rejected: Vec<RejectedRecipientInfo>,
}

impl OutboxError {
    pub(crate) fn reason(reason: &str) -> Self {
        Self {
            reason: reason.to_owned(),
            smtp_code: None,
            text: None,
            rejected: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServerReplyInfo {
    #[ts(type = "number")]
    pub code: u16,
    pub text: String,
}

/// One queue item as the outbox list and detail show it. No MIME.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItem {
    pub id: String,
    /// Cleared when the draft is deleted (after `accepted`, the item is the
    /// local Sent copy and outlives its draft).
    pub draft_id: Option<String>,
    pub account_id: String,
    /// With angle brackets.
    pub message_id: String,
    pub subject: String,
    /// The first few envelope recipients.
    pub recipients: Vec<String>,
    #[ts(type = "number")]
    pub recipient_count: u32,
    pub state: OutboxState,
    /// Earliest time of the first attempt (end of the undo window).
    pub not_before: String,
    pub data_started_at: Option<String>,
    /// Attempts started so far.
    #[ts(type = "number")]
    pub attempts: u32,
    pub next_attempt_at: Option<String>,
    pub first_queued_at: String,
    pub error: Option<OutboxError>,
    pub accepted_at: Option<String>,
    pub server_reply: Option<ServerReplyInfo>,
    /// The server's copy in a synced folder, once matched by Message-ID.
    pub server_copy_message_id: Option<String>,
    /// Set on an `unknown` item the user has settled; it is then not live.
    pub resolution: Option<OutboxResolution>,
    /// The item queued by `resend` for a settled `unknown` one.
    pub superseded_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Queues a draft for sending. Retrying with the same `clientMutationId`,
/// draft and revision returns the same item.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueueSendRequest {
    pub draft_id: String,
    #[ts(type = "number")]
    pub expected_revision: u32,
    pub client_mutation_id: String,
}

/// Settles an `unknown` item. `resend` queues a new attempt with the same
/// Message-ID and bytes; `markSent` declares the message delivered.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveUnknownRequest {
    pub id: String,
    pub resolution: UnknownResolution,
    pub client_mutation_id: String,
}

/// Emitted on every queue item state change. No addresses, subject or body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OutboxChanged {
    pub account_id: String,
    pub outbox_id: String,
    pub draft_id: Option<String>,
    pub state: OutboxState,
    #[ts(type = "number")]
    pub attempts: u32,
    pub next_attempt_at: Option<String>,
    /// The failure code when the item failed, was requeued after a failure or
    /// is `unknown`.
    pub error_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DraftChange {
    /// A draft was created (new, reply, forward).
    Created,
    /// The draft's content was saved.
    Updated,
    /// A queue item took the draft; it cannot be edited.
    Locked,
    /// The queue let go (canceled, failed); it is editable again.
    Unlocked,
    /// The draft was deleted because its message was accepted.
    Deleted,
}

/// A draft changed: created, edited, deleted, or locked / unlocked by the
/// send queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DraftChanged {
    pub draft_id: String,
    pub account_id: String,
    pub change: DraftChange,
}

impl From<&OutboxItem> for OutboxChanged {
    fn from(item: &OutboxItem) -> Self {
        Self {
            account_id: item.account_id.clone(),
            outbox_id: item.id.clone(),
            draft_id: item.draft_id.clone(),
            state: item.state,
            attempts: item.attempts,
            next_attempt_at: item.next_attempt_at.clone(),
            error_reason: item.error.as_ref().map(|error| error.reason.clone()),
        }
    }
}
