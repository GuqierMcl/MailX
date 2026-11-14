//! What the assistant exchanges with its host: run requests, snapshots and
//! events. Tool summaries are structured and independent of the model's
//! wording; raw tool output never appears here.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ErrorCode;

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
    /// Not run: the settings ask the user first (confirmation comes later) or
    /// the user declined.
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
    /// `count` emails were changed (flags, archive).
    Changed { count: u32, email_ids: Vec<String> },
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
    Finished {
        reason: FinishedReason,
    },
    Failed {
        code: ErrorCode,
    },
}
