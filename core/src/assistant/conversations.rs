//! Chat histories, in memory only. At most [`MAX_CONVERSATIONS`] are kept
//! (least recently used goes first) and a conversation takes one run at a
//! time. History is only replaced at consistent points (a user message, a
//! finished turn), so a stopped run never leaves a tool call without its
//! result behind.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use uuid::Uuid;

use crate::{AppError, ErrorCode, ai::client::stream::ChatMessage};

pub(crate) const MAX_CONVERSATIONS: usize = 20;

/// Said by the assistant in history when its reply was cut short.
pub(crate) const STOPPED_MARKER: &str = "[The reply was stopped before it finished.]";

struct Conversation {
    messages: Vec<ChatMessage>,
    active: bool,
    used: u64,
}

#[derive(Default)]
struct Table {
    map: HashMap<String, Conversation>,
    clock: u64,
}

#[derive(Default)]
pub(crate) struct Conversations {
    table: Mutex<Table>,
}

/// Held by a run for its conversation; dropping it (finish, failure or
/// cancel) frees the conversation for the next message.
pub(crate) struct ActiveConversation {
    store: Arc<Conversations>,
    pub(crate) id: String,
    /// Already released (rolled back); `drop` has nothing left to do.
    released: bool,
}

impl ActiveConversation {
    /// The run never started: takes the user message back out (a conversation
    /// that held only that message disappears) and frees the conversation.
    pub(crate) fn rollback(mut self) {
        self.released = true;
        let mut table = self.store.lock();
        if let Some(conversation) = table.map.get_mut(&self.id) {
            conversation.messages.pop();
            conversation.active = false;
            if conversation.messages.is_empty() {
                table.map.remove(&self.id);
            }
        }
    }
}

impl Drop for ActiveConversation {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let mut table = self.store.lock();
        if let Some(conversation) = table.map.get_mut(&self.id) {
            conversation.active = false;
            // History must end with the assistant, or the next request would
            // carry a tool call without an answer or two user turns in a row.
            if matches!(
                conversation.messages.last(),
                Some(ChatMessage::User(_) | ChatMessage::ToolResult { .. })
            ) {
                conversation.messages.push(ChatMessage::Assistant {
                    text: STOPPED_MARKER.to_owned(),
                    tool_calls: Vec::new(),
                });
            }
        }
    }
}

impl Conversations {
    fn lock(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens (`None`) or continues a conversation with a user message and
    /// returns its history including that message. An unknown id is
    /// `NotFound`, a conversation with a run is `AssistantBusy`.
    pub(crate) fn begin(
        self: &Arc<Self>,
        id: Option<&str>,
        message: String,
    ) -> Result<(ActiveConversation, Vec<ChatMessage>), AppError> {
        let mut table = self.lock();
        table.clock += 1;
        let clock = table.clock;
        let id = match id {
            Some(id) => {
                let conversation = table
                    .map
                    .get_mut(id)
                    .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
                if conversation.active {
                    return Err(AppError::new(ErrorCode::AssistantBusy));
                }
                conversation.active = true;
                conversation.used = clock;
                conversation.messages.push(ChatMessage::User(message));
                id.to_owned()
            }
            None => {
                if table.map.len() >= MAX_CONVERSATIONS {
                    let oldest = table
                        .map
                        .iter()
                        .filter(|(_, c)| !c.active)
                        .min_by_key(|(_, c)| c.used)
                        .map(|(id, _)| id.clone());
                    match oldest {
                        Some(oldest) => {
                            table.map.remove(&oldest);
                        }
                        None => return Err(AppError::new(ErrorCode::AssistantBusy)),
                    }
                }
                let id = Uuid::new_v4().to_string();
                table.map.insert(
                    id.clone(),
                    Conversation {
                        messages: vec![ChatMessage::User(message)],
                        active: true,
                        used: clock,
                    },
                );
                id
            }
        };
        let history = table.map[&id].messages.clone();
        drop(table);
        Ok((
            ActiveConversation {
                store: self.clone(),
                id: id.clone(),
                released: false,
            },
            history,
        ))
    }

    /// Replaces the history at a consistent point.
    pub(crate) fn commit(&self, id: &str, messages: &[ChatMessage]) {
        if let Some(conversation) = self.lock().map.get_mut(id) {
            conversation.messages = messages.to_vec();
        }
    }

    #[cfg(test)]
    pub(crate) fn history(&self, id: &str) -> Option<Vec<ChatMessage>> {
        self.lock().map.get(id).map(|c| c.messages.clone())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().map.len()
    }
}
