//! What the assistant exchanges with its host: run requests, snapshots and
//! events. Tool summaries are structured and independent of the model's
//! wording; raw tool output never appears here.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    DraftOrigin, ErrorCode, MailAddress,
    settings::{SummaryLanguage, SummaryStyle},
};

/// Longest chat message accepted, in characters.
pub const MAX_MESSAGE_CHARS: usize = 20_000;
/// Most selected emails a chat message can be about.
pub const MAX_CONTEXT_EMAILS: usize = 20;

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
        /// Skip the cached answer and ask the model again (the refresh
        /// button); the new answer replaces the cached one.
        #[serde(default)]
        fresh: bool,
        /// What today is on the user's calendar, when it is something: the
        /// interface works it out the same way as for the preset line. A closed
        /// list, never free text, so it cannot carry anything into the prompt.
        #[serde(default)]
        #[ts(optional)]
        occasion: Option<Occasion>,
    },
    /// One user message in a conversation; `conversationId` `None` opens a
    /// new conversation.
    Chat {
        #[serde(default)]
        #[ts(optional)]
        conversation_id: Option<String>,
        message: String,
        /// What the user is looking at as they write. The core reads it
        /// itself, within the assistant's accounts, for this run only.
        #[serde(default)]
        #[ts(optional)]
        context: Option<ChatContext>,
    },
    /// A summary of one email for the reader (`docs/architecture/ai-chat-ui.md`
    /// section 5.2). The core reads the email itself, within the assistant's
    /// accounts; its body must be on this device. A summary already made for
    /// the same body, style and language is answered from the local cache as
    /// `completed` at once, unless `fresh`.
    EmailSummary {
        email_id: String,
        #[serde(default)]
        #[ts(optional)]
        style: Option<SummaryStyle>,
        #[serde(default)]
        #[ts(optional)]
        language: Option<SummaryLanguage>,
        /// Skip the cached summary and ask the model again; the new one
        /// replaces it.
        #[serde(default)]
        #[ts(optional)]
        fresh: Option<bool>,
    },
}

/// A summary kept on this device for an email whose body has not changed
/// since.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CachedEmailSummary {
    pub style: SummaryStyle,
    /// A translation target code, or `original` for the email's own language.
    pub language: String,
    /// Untrusted model output; shown as Markdown without raw HTML.
    pub text: String,
    /// RFC 3339 UTC with milliseconds.
    pub created_at: String,
}

/// What a chat message is about: the mail or draft on screen. Only ids travel;
/// the core reads the content through the tool layer, so the account scope
/// applies and the interface cannot put text into the prompt.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChatContext {
    /// The email open in the reader.
    Email { email_id: String },
    /// Emails selected in the list, 1 to [`MAX_CONTEXT_EMAILS`].
    Emails { email_ids: Vec<String> },
    /// The draft being written or previewed.
    Draft { draft_id: String },
}

/// A holiday or solar term the Now page knows (`frontend/src/features/now/calendar.ts`);
/// the ids are the same. Solar terms are only sent for Chinese interfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Occasion {
    NewYear,
    SpringFestival,
    Lantern,
    DragonBoat,
    Qixi,
    MidAutumn,
    Xiaohan,
    Dahan,
    Lichun,
    Yushui,
    Jingzhe,
    Chunfen,
    Qingming,
    Guyu,
    Lixia,
    Xiaoman,
    Mangzhong,
    Xiazhi,
    Xiaoshu,
    Dashu,
    Liqiu,
    Chushu,
    Bailu,
    Qiufen,
    Hanlu,
    Shuangjiang,
    Lidong,
    Xiaoxue,
    Daxue,
    Dongzhi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RunKindTag {
    NowSummary,
    Chat,
    EmailSummary,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FinishedReason {
    /// The model gave its answer.
    Stop,
    /// The turn limit was reached; the user may continue.
    Limit,
    /// The model's output limit cut the answer off.
    Length,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FlagRestore {
    pub email_id: String,
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivity {
    pub call_id: String,
    pub name: String,
    pub status: ToolStatus,
    pub summary: ToolActivitySummary,
}

/// One stretch of the model's thinking in a run. A reply can have several
/// (one per model turn, e.g. before and after a tool call).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RunReasoning {
    /// The readable reasoning so far; empty when the server only sent
    /// redacted thinking.
    pub text: String,
    /// RFC 3339 UTC with milliseconds.
    pub started_at: String,
    /// `None` while it is still going.
    #[ts(type = "number | null")]
    pub duration_ms: Option<u64>,
    /// Where in `text` of the snapshot it began, in UTF-16 code units.
    pub text_offset: u32,
    /// How many of the run's `tools` began before it.
    pub tools_before: u32,
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
    /// The model's thinking, in order.
    pub reasoning: Vec<RunReasoning>,
    /// Whether this run may use tools. `false` for a chat on a model known to
    /// lack tool calling (it can only answer from the conversation) and for
    /// Now summaries; `true` for every other chat.
    pub tools_available: bool,
    pub error: Option<ErrorCode>,
    pub finished_reason: Option<FinishedReason>,
    /// When the run ended (`completed`, `failed` or `canceled`), RFC 3339 UTC
    /// with milliseconds; `None` while it is going. A Now summary answered
    /// from the cache carries the time its text was first generated.
    pub completed_at: Option<String>,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmAction {
    /// `outbox_send`, including a retry.
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
    /// The tool id, e.g. `outbox_send`.
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
    /// Thinking text. The first event of a `segment` (numbered from 0 in the
    /// run) opens it where the reply stands now; `delta` may be empty.
    Reasoning {
        segment: u32,
        delta: String,
    },
    /// The thinking stretch ended (text, a tool call or the end of the model
    /// turn followed).
    ReasoningEnd {
        segment: u32,
        #[ts(type = "number")]
        duration_ms: u64,
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

// ------------------------------------------------------- saved conversations

/// Page size of `list_conversations` when none is given, and its maximum.
pub const DEFAULT_CONVERSATION_PAGE: u32 = 50;
pub const MAX_CONVERSATION_PAGE: u32 = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    /// The first user message, shortened.
    pub title: String,
    /// RFC 3339 UTC with milliseconds: when the last turn was saved.
    pub updated_at: String,
    /// Messages the user sent.
    pub message_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConversationList {
    /// Newest first.
    pub items: Vec<ConversationSummary>,
    /// Pass back as `cursor` for the next page; `None` at the end.
    pub next_cursor: Option<String>,
}

/// How a saved reply ended. A reply cut off by a restart is `canceled`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TurnState {
    Completed,
    Failed,
    Canceled,
}

/// What the user answered to a confirmation (or did not).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmOutcome {
    Approved,
    Denied,
    /// Nobody answered in time; counts as declined.
    Expired,
}

/// A tool call of a saved reply. `textOffset` is where in the reply's text the
/// call began, in UTF-16 code units (what `String.length` counts), so the
/// interface can place it between the same words as it did live.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurnTool {
    pub call_id: String,
    pub name: String,
    pub status: ToolStatus,
    /// Never carries undo data: `changed.undo` is always `null`.
    pub summary: ToolActivitySummary,
    pub text_offset: u32,
}

/// A confirmation the user answered, as one line. The details card is not
/// kept. The call it belonged to is in `tools` under the same `callId`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurnConfirmation {
    pub call_id: String,
    pub tool: String,
    pub action: ConfirmAction,
    pub outcome: ConfirmOutcome,
    pub text_offset: u32,
}

/// A thinking stretch of a saved reply, placed like [`ChatTurnTool`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurnReasoning {
    /// Empty when the server only sent redacted thinking.
    pub text: String,
    #[ts(type = "number")]
    pub duration_ms: u64,
    pub text_offset: u32,
    /// How many of the reply's `tools` began before it.
    pub tools_before: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "role",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ChatTurn {
    User {
        text: String,
        /// RFC 3339 UTC with milliseconds.
        at: String,
    },
    /// Everything the assistant wrote and did in answer to one user message.
    Assistant {
        text: String,
        tools: Vec<ChatTurnTool>,
        reasoning: Vec<ChatTurnReasoning>,
        confirmations: Vec<ChatTurnConfirmation>,
        state: TurnState,
        /// Set when `state` is `completed`.
        finished_reason: Option<FinishedReason>,
        /// Set when `state` is `failed`.
        error: Option<ErrorCode>,
        at: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub turns: Vec<ChatTurn>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConversationChange {
    /// A turn was saved (the conversation is new or moved up the list).
    Saved,
    Deleted,
    /// Every saved conversation was deleted, or an account's were removed.
    Cleared,
}

/// Saved conversations changed; lists refetch. `conversationId` is `None`
/// for `cleared`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConversationsChanged {
    pub conversation_id: Option<String>,
    pub change: ConversationChange,
}
