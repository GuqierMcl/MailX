//! Mail read contract shared by local storage and host adapters. Folder and
//! per-folder header queries and cached body metadata are available. HTML and
//! inline resources remain behind the host-isolated body protocol.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub(crate) mod compatibility;
pub mod filename;
pub mod preview;
pub mod render;

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
    /// Whether a successful STATUS has supplied these counts. Unknown is not zero.
    pub counts_known: bool,
    /// Server unread count; for an inbox minus the locally archived unread mail.
    #[ts(type = "number")]
    pub unread_count: u32,
    #[ts(type = "number")]
    pub total_count: u32,
    /// Locally archived messages of this inbox (0 for other folders).
    #[ts(type = "number")]
    pub archived_count: u32,
    /// Locally archived unread messages of this inbox (0 for other folders).
    #[ts(type = "number")]
    pub archived_unread_count: u32,
}

/// A deletion MailX has accepted but not yet carried out on the server. The
/// messages are hidden already; until `undo_until` passes the user can take
/// it back with the batch id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailDeletion {
    pub batch_id: String,
    /// RFC 3339, UTC.
    pub undo_until: String,
    /// `true` when the messages are expunged for good instead of going to
    /// Trash (they were in Trash, or the account has none).
    pub permanent: bool,
    /// Ids skipped by a `partial` request, in input order; always empty
    /// otherwise.
    pub skipped_ids: Vec<String>,
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

/// Whether the list excerpt has been fetched. A full body download always wins:
/// `ready` can come from a partial fetch, `failed` means there was nothing to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PreviewState {
    Missing,
    Ready,
    Failed,
}

/// Controlled reasons only; server text and local paths are never persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum BodyFailureCode {
    TooLarge,
    Stale,
    Network,
    Authentication,
    ProviderPolicy,
    Protocol,
    Storage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailSummary {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub subject: String,
    pub from: MailAddress,
    /// Leading recipients for Sent-style rows (at most three) and the total.
    pub to: EmailRecipients,
    /// Short plain-text excerpt for list rows, never HTML.
    pub preview: String,
    /// RFC 3339 timestamp.
    pub received_at: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachment: bool,
    pub body_state: BodyState,
    pub preview_state: PreviewState,
    /// Local changes (read, star, move) not yet confirmed by the server.
    pub pending_changes: bool,
    /// A copy of an accepted send kept only on this device (`local-sent:` ids).
    pub local_only: bool,
    /// Archived on this device only (inbox messages; the server still has it in INBOX).
    pub archived: bool,
    /// Plain-text excerpt around a full-text hit in the body or attachment
    /// names; `None` when there is no text search or the hit is only in the
    /// subject or people.
    pub match_snippet: Option<Vec<SnippetSegment>>,
    /// Set when this row stands for every message of an aggregated sender in
    /// the list (see [`EmailListQuery::group_senders`]); the row itself is
    /// that sender's newest message in scope.
    pub sender_group: Option<SenderGroup>,
}

/// A sender whose messages are collapsed into one list row. Counts cover the
/// messages that pass every filter of the query, across all accounts in scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SenderGroup {
    /// Normalized (ASCII lowercase) sender address.
    pub address: String,
    #[ts(type = "number")]
    pub total: u32,
    #[ts(type = "number")]
    pub unread: u32,
}

/// An aggregated sender, as listed in the contact settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ContactAggregation {
    /// Normalized (ASCII lowercase) sender address.
    pub address: String,
    /// Display name of the newest visible message from this address.
    pub name: Option<String>,
    /// Visible messages from this address in every account, outside junk and trash.
    #[ts(type = "number")]
    pub message_count: u32,
    pub latest_received_at: Option<String>,
    pub created_at: String,
}

/// One run of a search snippet; `hit` marks the matched text. Plain text only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnippetSegment {
    pub text: String,
    pub hit: bool,
}

/// First recipients of a message and how many there are in total.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailRecipients {
    /// At most three addresses.
    pub addresses: Vec<MailAddress>,
    #[ts(type = "number")]
    pub total: u32,
}

/// Prefix of the ids of local Sent copies (sending.md section 9.2).
pub const LOCAL_SENT_PREFIX: &str = "local-sent:";

/// The most messages one "select all" returns, matching the batch action limit.
pub const MAX_SELECTION: u32 = 500;

/// Which locally archived inbox messages a list contains. This is the local
/// archive, not the server's `FolderRole::Archive` folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveScope {
    /// Hide archived messages (inbox views).
    Exclude,
    /// Only archived messages (the archived views).
    Only,
    /// Both; archived rows carry `archived: true`.
    Include,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct EmailListQuery {
    /// `None` searches every account.
    pub account_ids: Option<Vec<String>>,
    pub folder_id: Option<String>,
    /// Matches folders with this role in every selected account, e.g. a unified inbox.
    pub folder_role: Option<FolderRole>,
    /// Folders with these roles are left out, e.g. junk and trash in the
    /// Starred view. Only narrows the result; an explicit `folder_role` is
    /// not affected by it. Empty (the default) excludes nothing.
    pub exclude_folder_roles: Vec<FolderRole>,
    /// `None` hides archived messages, except in a text search, which finds them.
    pub archive: Option<ArchiveScope>,
    pub text: Option<String>,
    pub unread: Option<bool>,
    /// Match the local desired star state, including pending server writes.
    pub starred: Option<bool>,
    pub has_attachment: Option<bool>,
    /// Only messages from this sender address (case-insensitive). Local Sent
    /// copies match on the account's own address.
    pub from_address: Option<String>,
    /// Collapse messages of aggregated senders into one row each. Ignored in
    /// a text search and together with `from_address`.
    pub group_senders: bool,
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
    /// At least one matching selectable folder exceeded the per-round UID limit.
    pub window_capped: bool,
    pub bodies_complete: bool,
    /// Messages in scope whose body text is in the local search index.
    #[ts(type = "number")]
    pub indexed_bodies: u64,
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

/// One message of a "select all": just enough to decide which batch actions
/// apply and whether deleting needs confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SelectedEmail {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub archived: bool,
    pub local_only: bool,
}

/// The first messages of a list query, in list order; `truncated` says more
/// matched than were returned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailSelection {
    pub items: Vec<SelectedEmail>,
    pub truncated: bool,
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

/// Safe reading metadata. HTML is served only by the isolated body protocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EmailBody {
    pub text: Option<String>,
    pub has_html: bool,
    pub truncated: bool,
    pub remote_images_blocked: u32,
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
    pub body_failure: Option<BodyFailureCode>,
    pub attachments: Vec<AttachmentSummary>,
}
