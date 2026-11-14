//! What the assistant exchanges with its host: run requests, snapshots and
//! events. Tool summaries are structured and independent of the model's
//! wording; raw tool output never appears here.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{DraftOrigin, ErrorCode, MailAddress};

/// Longest chat message accepted, in characters.
pub const MAX_MESSAGE_CHARS: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRun {
    pub kind: RunKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RunKind {
    /// The one or two sentences on the Now page. `since` and `dayStarts` are
    /// the overview query (`MailOverviewQuery`); the core computes the
    /// overview itself, so the interface sends no mail facts.
    NowSummary {
        since: Option<String>,
        #[serde(default)]
        #[ts(optional)]
        day_starts: Option<Vec<String>>,
    },
    /// One user message in a conversation; `conversationId` `None` opens a
    /// new conversation.
    Chat {
        #[serde(default)]
        #[ts(optional)]
        conversation_id: Option<String>,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RunKindTag {
    NowSummary,
    Chat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    /// The user has not agreed to send mail content to the model's
    /// connection. Nothing was sent; consent, then start again.
    AwaitingConsent,
    Running,
    /// Paused until the main window answers a confirmation request.
    AwaitingConfirmation,
    Completed,
    Failed,
    Canceled,
}

impl RunState {
    /// Counts against the limit of simultaneous runs.
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::AwaitingConfirmation)
    }

    /// Nothing more will happen to the run.
    pub fn finished(self) -> bool {
        !self.active()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FinishedReason {
    /// The model gave its answer.
    Stop,
    /// The turn limit was reached; the user may continue.
    Limit,
    /// The model's output limit cut the answer off.
    Length,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
    /// Not run: the user declined, the confirmation timed out, or the same
    /// call was already declined in this run.
    Denied,
}

/// A short, model-independent account of one tool call for the interface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolActivitySummary {
    /// Not finished yet.
    Pending,
    /// A search or list that found `count` items.
    Found { count: u32 },
    /// Emails the model read.
    Read { email_ids: Vec<String> },
    /// `count` emails were changed (flags, archive). `undo` is what the
    /// trusted UI needs to take the change back; it is never shown to the
    /// model.
    Changed {
        count: u32,
        email_ids: Vec<String>,
        undo: Option<ToolUndo>,
    },
    /// A draft was created or changed.
    Draft { draft_id: String, subject: String },
    /// A send was queued, or a queued send changed.
    Outbox { outbox_id: String },
    /// Finished; nothing more to say.
    Done,
    /// Not run, see [`ToolStatus::Denied`].
    Denied,
    /// The tool's stable error code.
    Error { code: String },
}

/// How the UI takes back an organize call, through commands it already has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolUndo {
    /// The one-use token for `undo_email_archive`; it expires with the
    /// core's receipt (ten minutes) and after any later change to the same
    /// mail.
    Archive { token: String },
    /// Flags before the call, for the existing flag command. A field is
    /// `None` when the call did not change it.
    Flags { restore: Vec<FlagRestore> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FlagRestore {
    pub email_id: String,
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivity {
    pub call_id: String,
    pub name: String,
    pub status: ToolStatus,
    pub summary: ToolActivitySummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshot {
    pub run_id: String,
    pub kind: RunKindTag,
    pub state: RunState,
    /// Everything the model has written so far.
    pub text: String,
    pub tools: Vec<ToolActivity>,
    pub error: Option<ErrorCode>,
    pub finished_reason: Option<FinishedReason>,
    /// The `seq` of the last event this snapshot includes; later events
    /// continue from it.
    #[ts(type = "number")]
    pub seq: u64,
    /// Set for chats.
    pub conversation_id: Option<String>,
    /// The question the run waits on; set exactly while `state` is
    /// `awaitingConfirmation`.
    pub pending: Option<PendingConfirmation>,
}

/// A tool call waiting for the user's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PendingConfirmation {
    pub request_id: String,
    pub call_id: String,
    pub request: ConfirmationRequest,
}

/// What kind of step needs the user. The labels follow the settings
/// categories in `docs/architecture/ai-permissions.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmAction {
    /// `send_email`, including a retry.
    Send,
    CancelSend,
    Organize,
    /// Creating a draft, or changing one the assistant created.
    AiDraft,
    /// Changing a draft the user (or MCP) created.
    UserDraft,
}

/// Why a send deserves a second look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmWarning {
    /// Some recipient never appears on mail of this account in the local
    /// cache.
    NewRecipient,
    /// Some recipient is not on any mail the assistant read or searched in
    /// this run.
    NotInConversation,
    HasAttachments,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmAttachment {
    pub file_name: String,
    #[ts(type = "number")]
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OrganizeChange {
    Archive,
    Unarchive,
    /// Only the flags the call sets are present.
    Flags {
        is_read: Option<bool>,
        is_starred: Option<bool>,
    },
    Refresh,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConfirmDraftChange {
    Create,
    /// Names of the fields the call replaces: `to`, `cc`, `bcc`, `subject`,
    /// `body`.
    Update {
        fields: Vec<String>,
    },
    Delete,
    AttachmentAdded {
        file_name: String,
    },
    AttachmentRemoved {
        file_name: String,
    },
}

/// The facts a confirmation card shows. Built by the core from current data;
/// nothing in it is text the model wrote about its own action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConfirmationDetails {
    /// A draft about to be queued for sending.
    Send {
        account_name: String,
        account_address: String,
        to: Vec<MailAddress>,
        cc: Vec<MailAddress>,
        bcc: Vec<MailAddress>,
        subject: String,
        /// The first characters of the plain-text body.
        body_preview: String,
        attachments: Vec<ConfirmAttachment>,
        warnings: Vec<ConfirmWarning>,
    },
    /// A queued send: retried now (`retry`) or canceled.
    Outbox {
        retry: bool,
        sender_address: String,
        subject: String,
        /// The first few recipients; `recipientCount` counts all.
        recipients: Vec<String>,
        recipient_count: u32,
    },
    Organize {
        change: OrganizeChange,
        /// Emails the call targets that exist and are in scope.
        count: u32,
        /// Subjects of up to five of them.
        subjects: Vec<String>,
    },
    Draft {
        subject: String,
        origin: DraftOrigin,
        change: ConfirmDraftChange,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationRequest {
    pub action: ConfirmAction,
    /// The tool id, e.g. `send_email`.
    pub tool: String,
    pub details: ConfirmationDetails,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEvent {
    pub run_id: String,
    /// Increases by one per event of a run, starting at 1.
    #[ts(type = "number")]
    pub seq: u64,
    pub kind: AssistantEventKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AssistantEventKind {
    State {
        state: RunState,
    },
    Text {
        delta: String,
    },
    Tool {
        call_id: String,
        name: String,
        status: ToolStatus,
        summary: ToolActivitySummary,
    },
    /// The run paused for the user; answer with `respond`. Follows the
    /// `awaitingConfirmation` state event.
    Confirm {
        request_id: String,
        call_id: String,
        request: ConfirmationRequest,
    },
    Finished {
        reason: FinishedReason,
    },
    Failed {
        code: ErrorCode,
    },
}
