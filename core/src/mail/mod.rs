//! Mail read contract shared by local storage and host adapters. Folder and
//! inbox header queries are implemented; bodies and attachment contents remain
//! unavailable until their retrieval is implemented.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Archive,
    Junk,
    Trash,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub role: FolderRole,
    /// `false` for IMAP `\Noselect` containers that only group child folders.
    pub selectable: bool,
    #[ts(type = "number")]
    pub unread_count: u32,
    #[ts(type = "number")]
    pub total_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MailAddress {
    pub name: Option<String>,
    pub address: String,
}

/// Whether the message body is available locally.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum BodyState {
    Missing,
    Fetching,
    Ready,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailSummary {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub subject: String,
    pub from: MailAddress,
    /// Short plain-text excerpt for list rows, never HTML.
    pub preview: String,
    /// RFC 3339 timestamp.
    pub received_at: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachment: bool,
    pub body_state: BodyState,
    /// Local changes (read, star, move) not yet confirmed by the server.
    pub pending_changes: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct EmailListQuery {
    /// `None` searches every account.
    pub account_ids: Option<Vec<String>>,
    pub folder_id: Option<String>,
    /// Matches folders with this role in every selected account, e.g. a unified inbox.
    pub folder_role: Option<FolderRole>,
    pub text: Option<String>,
    pub unread: Option<bool>,
    pub has_attachment: Option<bool>,
    pub received_after: Option<String>,
    pub received_before: Option<String>,
    #[ts(type = "number | null")]
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// How much of the queried range is cached locally, so empty or short results
/// are not mistaken for the server having no mail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub headers_complete: bool,
    pub bodies_complete: bool,
    pub synced_after: Option<String>,
    pub synced_before: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailPage {
    pub items: Vec<EmailSummary>,
    pub next_cursor: Option<String>,
    pub coverage: Coverage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentSummary {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    #[ts(type = "number")]
    pub size: u64,
    pub is_inline: bool,
}

/// Body parts as received. `html` is untrusted and must only be rendered in an
/// isolated, script-free frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailBody {
    pub text: Option<String>,
    pub html: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailDetail {
    pub summary: EmailSummary,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub reply_to: Vec<MailAddress>,
    /// `None` until `summary.body_state` is `ready`.
    pub body: Option<EmailBody>,
    pub attachments: Vec<AttachmentSummary>,
}
