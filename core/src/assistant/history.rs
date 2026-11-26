//! Saved conversations: the stored row format, reading them back into the
//! model's history and into turns for the interface, and the save itself.
//!
//! Rows (`assistant_messages`) mirror the in-memory history one to one:
//! `user`, `assistant` (text, tool calls and the model's reasoning: the data
//! to send back plus how long each stretch took), `tool` (a tool result with the
//! activity the interface showed and, when the user was asked, the outcome),
//! plus an `end` row after each finished reply (how it ended). Tool results
//! are stored as they went to the model, so they already carry the loop's
//! per-result limit. Undo data and open confirmations are never stored.
use std::{collections::BTreeSet, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    Inner,
    conversations::{STOPPED_MARKER, ToolRecord},
    runs::now_rfc3339,
    types::{
        ChatTurn, ChatTurnConfirmation, ChatTurnReasoning, ChatTurnTool, ConfirmAction,
        ConfirmOutcome, ConversationChange, ConversationsChanged, FinishedReason, ToolActivity,
        ToolActivitySummary, TurnState,
    },
};
use crate::{
    AppError, CoreEvent, ErrorCode,
    ai::{
        client::stream::{ChatMessage, ToolCallRequest},
        reasoning::AssistantReasoning,
    },
    storage::assistant_chats::{Append, NewRow, StoredRow},
};

/// Title length, in characters.
const TITLE_CHARS: usize = 60;

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Content {
    User {
        text: String,
    },
    Assistant {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<Call>,
        /// Absent in rows written before reasoning was kept.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<AssistantReasoning>,
    },
    Tool {
        call_id: String,
        content: String,
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity: Option<ToolActivity>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirmation: Option<Confirmation>,
    },
    End {
        state: TurnState,
        #[serde(default)]
        finished_reason: Option<FinishedReason>,
        #[serde(default)]
        error: Option<ErrorCode>,
    },
}

#[derive(Serialize, Deserialize)]
struct Call {
    id: String,
    name: String,
    arguments: Value,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Confirmation {
    action: ConfirmAction,
    outcome: ConfirmOutcome,
}

/// How a reply ended, written as its closing row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct End {
    pub state: TurnState,
    pub finished_reason: Option<FinishedReason>,
    pub error: Option<ErrorCode>,
}

impl End {
    pub(crate) fn finished(reason: FinishedReason) -> Self {
        Self {
            state: TurnState::Completed,
            finished_reason: Some(reason),
            error: None,
        }
    }

    pub(crate) fn failed(code: ErrorCode) -> Self {
        Self {
            state: TurnState::Failed,
            finished_reason: None,
            error: Some(code),
        }
    }

    pub(crate) fn canceled() -> Self {
        Self {
            state: TurnState::Canceled,
            finished_reason: None,
            error: None,
        }
    }
}

/// The first user message on one line, at most [`TITLE_CHARS`] characters.
pub(crate) fn title(message: &str) -> String {
    let line = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TITLE_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(TITLE_CHARS).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}

/// The activity as it is stored: without anything that could undo a change.
fn without_undo(mut activity: ToolActivity) -> ToolActivity {
    if let ToolActivitySummary::Changed { undo, .. } = &mut activity.summary {
        *undo = None;
    }
    activity
}

fn row(role: &'static str, content: &Content, at: &str) -> NewRow {
    NewRow {
        role,
        // Plain data structures: serializing cannot fail.
        content: serde_json::to_string(content).unwrap_or_default(),
        created_at: at.to_owned(),
    }
}

pub(crate) fn message_row(message: &ChatMessage, record: Option<&ToolRecord>, at: &str) -> NewRow {
    match message {
        ChatMessage::User(text) => row("user", &Content::User { text: text.clone() }, at),
        ChatMessage::Assistant {
            text,
            tool_calls,
            reasoning,
        } => row(
            "assistant",
            &Content::Assistant {
                reasoning: reasoning.clone(),
                text: text.clone(),
                tool_calls: tool_calls
                    .iter()
                    .map(|call| Call {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .collect(),
            },
            at,
        ),
        ChatMessage::ToolResult {
            call_id,
            content,
            is_error,
        } => row(
            "tool",
            &Content::Tool {
                call_id: call_id.clone(),
                content: content.clone(),
                is_error: *is_error,
                activity: record.map(|r| without_undo(r.activity.clone())),
                confirmation: record
                    .and_then(|r| r.confirmation)
                    .map(|(action, outcome)| Confirmation { action, outcome }),
            },
            at,
        ),
    }
}

pub(crate) fn end_row(end: &End, at: &str) -> NewRow {
    row(
        "end",
        &Content::End {
            state: end.state,
            finished_reason: end.finished_reason,
            error: end.error,
        },
        at,
    )
}

/// A conversation read back for continuing.
pub(crate) struct Loaded {
    pub messages: Vec<ChatMessage>,
    pub stamps: Vec<String>,
    /// Rows read, including closing rows.
    pub rows: u32,
    /// The row of each user message, by its index in `messages`.
    pub user_rows: std::collections::BTreeMap<usize, u32>,
}

/// `StorageIncompatible` when a row cannot be read.
pub(crate) fn load(rows: &[StoredRow]) -> Result<Loaded, AppError> {
    let mut messages = Vec::new();
    let mut stamps = Vec::new();
    let mut user_rows = std::collections::BTreeMap::new();
    for (row, stored) in rows.iter().enumerate() {
        let content: Content = serde_json::from_str(&stored.content)
            .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
        let message = match content {
            Content::User { text } => {
                if let Ok(row) = u32::try_from(row) {
                    user_rows.insert(messages.len(), row);
                }
                ChatMessage::User(text)
            }
            Content::Assistant {
                text,
                tool_calls,
                reasoning,
            } => ChatMessage::Assistant {
                reasoning,
                text,
                tool_calls: tool_calls
                    .into_iter()
                    .map(|call| ToolCallRequest {
                        id: call.id,
                        name: call.name,
                        arguments: call.arguments,
                    })
                    .collect(),
            },
            Content::Tool {
                call_id,
                content,
                is_error,
                ..
            } => ChatMessage::ToolResult {
                call_id,
                content,
                is_error,
            },
            Content::End { .. } => continue,
        };
        messages.push(message);
        stamps.push(stored.created_at.clone());
    }
    Ok(Loaded {
        messages,
        stamps,
        rows: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        user_rows,
    })
}

// -------------------------------------------------------------------- turns

/// An assistant reply being put together from rows.
#[derive(Default)]
struct Open {
    text: String,
    /// `text` in UTF-16 code units.
    units: u32,
    offsets: std::collections::HashMap<String, u32>,
    tools: Vec<ChatTurnTool>,
    reasoning: Vec<ChatTurnReasoning>,
    confirmations: Vec<ChatTurnConfirmation>,
    end: Option<End>,
    at: String,
}

impl Open {
    fn close(self) -> ChatTurn {
        let end = self.end.unwrap_or_else(End::canceled);
        ChatTurn::Assistant {
            text: self.text,
            tools: self.tools,
            reasoning: self.reasoning,
            confirmations: self.confirmations,
            state: end.state,
            finished_reason: end.finished_reason,
            error: end.error,
            at: self.at,
        }
    }
}

/// The conversation as the interface shows it: one user turn per user message
/// and one assistant turn for everything up to the next one. A reply with no
/// closing row (the app was closed during it) is `canceled`. Rows that cannot
/// be read are skipped.
pub(crate) fn turns(rows: &[StoredRow]) -> Vec<ChatTurn> {
    let mut out = Vec::new();
    let mut open: Option<Open> = None;
    for stored in rows {
        let Ok(content) = serde_json::from_str::<Content>(&stored.content) else {
            continue;
        };
        match content {
            Content::User { text } => {
                out.extend(open.take().map(Open::close));
                out.push(ChatTurn::User {
                    text,
                    at: stored.created_at.clone(),
                });
            }
            Content::Assistant {
                text,
                tool_calls,
                reasoning,
            } => {
                let reply = open.get_or_insert_with(Open::default);
                for (span, shown) in reasoning.as_ref().map(|r| r.shown()).unwrap_or_default() {
                    let before = text.get(..span.text_at).unwrap_or(&text);
                    let units = u32::try_from(before.encode_utf16().count()).unwrap_or(0);
                    let tools = u32::try_from(span.tools_before).unwrap_or(u32::MAX);
                    reply.reasoning.push(ChatTurnReasoning {
                        text: shown,
                        duration_ms: span.duration_ms,
                        text_offset: reply.units.saturating_add(units),
                        // The rows of earlier calls came before this one.
                        tools_before: u32::try_from(reply.tools.len())
                            .unwrap_or(u32::MAX)
                            .saturating_add(tools),
                    });
                }
                if text != STOPPED_MARKER {
                    reply.units = reply
                        .units
                        .saturating_add(u32::try_from(text.encode_utf16().count()).unwrap_or(0));
                    reply.text.push_str(&text);
                }
                for call in tool_calls {
                    reply.offsets.insert(call.id, reply.units);
                }
                reply.at.clone_from(&stored.created_at);
            }
            Content::Tool {
                call_id,
                activity,
                confirmation,
                ..
            } => {
                let reply = open.get_or_insert_with(Open::default);
                let offset = reply.offsets.get(&call_id).copied().unwrap_or(reply.units);
                if let Some(activity) = activity {
                    if let Some(Confirmation { action, outcome }) = confirmation {
                        reply.confirmations.push(ChatTurnConfirmation {
                            call_id: call_id.clone(),
                            tool: activity.name.clone(),
                            action,
                            outcome,
                            text_offset: offset,
                        });
                    }
                    let activity = without_undo(activity);
                    reply.tools.push(ChatTurnTool {
                        call_id,
                        name: activity.name,
                        status: activity.status,
                        summary: activity.summary,
                        text_offset: offset,
                    });
                }
                reply.at.clone_from(&stored.created_at);
            }
            Content::End {
                state,
                finished_reason,
                error,
            } => {
                let mut reply = open.take().unwrap_or_default();
                reply.end = Some(End {
                    state,
                    finished_reason,
                    error,
                });
                reply.at.clone_from(&stored.created_at);
                out.push(reply.close());
            }
        }
    }
    out.extend(open.map(Open::close));
    out
}

// ------------------------------------------------------------ touched accounts

/// Account ids a tool call named in its input or returned in its result
/// (`accountId` / `accountIds`), so removing an account can delete the
/// conversations that dealt with it.
pub(crate) fn touched_accounts(input: &Value, output: &Value, out: &mut BTreeSet<String>) {
    fn walk(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (key, inner) in map {
                    match (key.as_str(), inner) {
                        ("accountId", Value::String(id)) => add(id, out),
                        ("accountIds", Value::Array(ids)) => {
                            ids.iter()
                                .filter_map(Value::as_str)
                                .for_each(|id| add(id, out));
                        }
                        _ => walk(inner, out),
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    fn add(id: &str, out: &mut BTreeSet<String>) {
        if let Ok(id) = uuid::Uuid::parse_str(id) {
            out.insert(id.to_string());
        }
    }
    walk(input, out);
    walk(output, out);
}

// --------------------------------------------------------------------- save

/// Writes what the conversation has not saved yet, with `end` as the closing
/// row of a finished reply. Best effort: a failed write is retried by the
/// next save, and the chat itself never depends on it. Does nothing while
/// the history setting is off (and never saves that conversation after).
pub(crate) async fn persist(inner: &Arc<Inner>, id: &str, end: Option<End>) {
    let keep = inner
        .settings
        .get()
        .is_ok_and(|snapshot| snapshot.settings.assistant.save_history);
    if !keep {
        inner.conversations.stop_saving(id);
        return;
    }
    // Deleting a conversation takes the same gate, so a save cannot bring a
    // deleted conversation back.
    let _gate = inner.save_gate.lock().await;
    let Some(batch) = inner.conversations.unsaved(id, end.as_ref()) else {
        return;
    };
    let core = inner.core.clone();
    let result = tokio::task::spawn_blocking(move || {
        let now = now_rfc3339();
        let outcome = core.accounts().and_then(|mut service| {
            service.assistant_chat_append(&Append {
                id: &batch.id,
                create: batch.create.as_deref(),
                from_seq: batch.from_row,
                rows: &batch.rows,
                accounts: &batch.accounts,
                now: &now,
            })
        });
        (batch, outcome)
    })
    .await;
    match result {
        Ok((batch, Ok(true))) => {
            inner.conversations.mark_saved(&batch);
            inner.events.emit(&CoreEvent::AssistantConversationsChanged(
                ConversationsChanged {
                    conversation_id: Some(batch.id.clone()),
                    change: ConversationChange::Saved,
                },
            ));
        }
        // Deleted (or pruned, or its account removed) in the meantime.
        Ok((batch, Ok(false))) => inner.conversations.stop_saving(&batch.id),
        _ => {}
    }
}

/// Holds a run's conversation and saves its end. [`SaveGuard::finish`] is the
/// normal end; if the run is dropped instead (cancel, panic) the guard saves
/// it as stopped.
pub(crate) struct SaveGuard {
    inner: Arc<Inner>,
    conversation: Option<super::conversations::ActiveConversation>,
}

impl SaveGuard {
    pub(crate) fn new(
        inner: Arc<Inner>,
        conversation: super::conversations::ActiveConversation,
    ) -> Self {
        Self {
            inner,
            conversation: Some(conversation),
        }
    }

    pub(crate) fn id(&self) -> String {
        self.conversation
            .as_ref()
            .map(|c| c.id.clone())
            .unwrap_or_default()
    }

    /// Saves the reply with its ending and frees the conversation. The save
    /// runs in its own task, so a cancel arriving now cannot cut it short.
    pub(crate) async fn finish(&mut self, end: End) {
        if let Some(task) = self.save(end) {
            let _ = task.await;
        }
    }

    fn save(&mut self, end: End) -> Option<tokio::task::JoinHandle<()>> {
        let conversation = self.conversation.take()?;
        conversation.seal();
        let inner = self.inner.clone();
        let handle = tokio::runtime::Handle::try_current().ok()?;
        Some(handle.spawn(async move {
            persist(&inner, &conversation.id, Some(end)).await;
            drop(conversation);
        }))
    }
}

impl Drop for SaveGuard {
    fn drop(&mut self) {
        drop(self.save(End::canceled()));
    }
}

#[cfg(test)]
mod tests;
