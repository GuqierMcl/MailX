//! Draft wire contract. Drafts are local only; nothing here reaches a server.
use crate::mail::MailAddress;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DraftKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl DraftKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Reply => "reply",
            Self::ReplyAll => "replyAll",
            Self::Forward => "forward",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "new" => Self::New,
            "reply" => Self::Reply,
            "replyAll" => Self::ReplyAll,
            "forward" => Self::Forward,
            _ => return None,
        })
    }
}

/// A plain-text message being written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    pub account_id: String,
    pub kind: DraftKind,
    /// The message this draft answers or forwards. Cleared if that message
    /// disappears; the quoted text and headers were copied at creation.
    pub source_email_id: Option<String>,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub subject: String,
    pub body_text: String,
    /// Original `Message-ID` in angle brackets, for replies.
    pub in_reply_to: Option<String>,
    /// Space-separated `<message-id>` list, for replies.
    pub references: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Advances on every saved change; updates must present the one they saw.
    #[ts(type = "number")]
    pub revision: u32,
}

/// One row of the draft list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DraftSummary {
    pub id: String,
    pub account_id: String,
    pub kind: DraftKind,
    pub source_email_id: Option<String>,
    pub to: Vec<MailAddress>,
    pub subject: String,
    /// Start of the body on one line.
    pub preview: String,
    pub updated_at: String,
    #[ts(type = "number")]
    pub revision: u32,
}

/// Creates an empty or pre-filled new draft. Retrying with the same
/// `clientMutationId` returns the same draft.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDraftRequest {
    /// `null` picks the default sender account, else the first account.
    pub account_id: Option<String>,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub subject: String,
    pub body_text: String,
    pub client_mutation_id: String,
}

/// Creates a reply, reply-all or forward draft from a cached message. The
/// message body must already be cached.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDraftFromRequest {
    pub email_id: String,
    /// `reply`, `replyAll` or `forward`.
    pub kind: DraftKind,
    pub client_mutation_id: String,
}

/// Replaces the draft's content. Fails with `conflict` unless
/// `expectedRevision` is the current revision.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDraftRequest {
    pub id: String,
    #[ts(type = "number")]
    pub expected_revision: u32,
    /// `null` keeps the current sender account.
    pub account_id: Option<String>,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub subject: String,
    pub body_text: String,
}
