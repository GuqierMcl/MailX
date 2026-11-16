//! Chat histories: the working set in memory, mirrored to storage by
//! `history` when the user keeps history. At most [`MAX_CONVERSATIONS`] are in
//! memory (least recently used goes first) and a conversation takes one run at
//! a time. History is only replaced at consistent points (a user message, a
//! finished turn), so a stopped run never leaves a tool call without its
//! result behind.
//!
//! Next to the model's history each conversation remembers how much of it is
//! already saved (`saved_messages` messages, `saved_rows` rows including the
//! closing rows of finished replies), the tool activity that goes with each
//! tool result, and the accounts its tool calls touched.
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use uuid::Uuid;

use super::{
    history::{self, End, Loaded},
    runs::now_rfc3339,
    types::{ConfirmAction, ConfirmOutcome, ToolActivity},
};
use crate::{AppError, ErrorCode, ai::client::stream::ChatMessage, storage::assistant_chats};

pub(crate) const MAX_CONVERSATIONS: usize = 20;
/// A saved conversation holds at most this many messages (model history
/// entries) when a new user message is added; a run in progress may go past
/// it by what one message needs. Past it the conversation is full.
pub(crate) const MAX_MESSAGES: usize = 400;

/// Said by the assistant in history when its reply was cut short.
pub(crate) const STOPPED_MARKER: &str = "[The reply was stopped before it finished.]";

/// What the interface shows for one tool result, kept for saving.
#[derive(Clone, Debug)]
pub(crate) struct ToolRecord {
    pub activity: ToolActivity,
    pub confirmation: Option<(ConfirmAction, ConfirmOutcome)>,
}

/// What one run learned that history alone does not hold.
#[derive(Clone, Debug, Default)]
pub(crate) struct RunExtras {
    /// By index of the tool result in the conversation's history.
    pub records: BTreeMap<usize, ToolRecord>,
    pub accounts: BTreeSet<String>,
}

struct Conversation {
    messages: Vec<ChatMessage>,
    /// RFC 3339 time of each message; same length as `messages`.
    stamps: Vec<String>,
    active: bool,
    used: u64,
    saved_messages: usize,
    saved_rows: u32,
    /// The header row exists in storage.
    in_store: bool,
    /// Never save this conversation again (history was turned off while it
    /// ran, or it was deleted from storage).
    no_save: bool,
    records: BTreeMap<usize, ToolRecord>,
    accounts: BTreeSet<String>,
}

impl Conversation {
    fn fresh(message: String, clock: u64) -> Self {
        Self {
            messages: vec![ChatMessage::User(message)],
            stamps: vec![now_rfc3339()],
            active: true,
            used: clock,
            saved_messages: 0,
            saved_rows: 0,
            in_store: false,
            no_save: false,
            records: BTreeMap::new(),
            accounts: BTreeSet::new(),
        }
    }

    /// History must end with the assistant, or the next request would carry a
    /// tool call without an answer or two user turns in a row.
    fn seal(&mut self) {
        if matches!(
            self.messages.last(),
            Some(ChatMessage::User(_) | ChatMessage::ToolResult { .. })
        ) {
            self.messages.push(ChatMessage::Assistant {
                text: STOPPED_MARKER.to_owned(),
                tool_calls: Vec::new(),
            });
            self.stamps.push(now_rfc3339());
        }
    }
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

/// What a save writes, taken from memory at one moment.
pub(crate) struct Batch {
    pub id: String,
    /// The title when this is the conversation's first save.
    pub create: Option<String>,
    pub from_row: u32,
    pub rows: Vec<assistant_chats::NewRow>,
    pub accounts: Vec<String>,
    messages_after: usize,
    rows_after: u32,
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
            conversation.stamps.pop();
            conversation.active = false;
            if conversation.messages.is_empty() {
                table.map.remove(&self.id);
            }
        }
    }

    /// Makes history end with the assistant now; the conversation stays held.
    /// Saving the run's last rows needs this before the guard lets go.
    pub(crate) fn seal(&self) {
        if let Some(conversation) = self.store.lock().map.get_mut(&self.id) {
            conversation.seal();
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
            conversation.seal();
        }
    }
}

impl Conversations {
    fn lock(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes room for one more conversation by dropping the least recently
    /// used one that has no run. `AssistantBusy` when all of them have.
    fn make_room(table: &mut Table) -> Result<(), AppError> {
        if table.map.len() < MAX_CONVERSATIONS {
            return Ok(());
        }
        let oldest = table
            .map
            .iter()
            .filter(|(_, c)| !c.active)
            .min_by_key(|(_, c)| c.used)
            .map(|(id, _)| id.clone());
        match oldest {
            Some(oldest) => {
                table.map.remove(&oldest);
                Ok(())
            }
            None => Err(AppError::new(ErrorCode::AssistantBusy)),
        }
    }

    /// Opens (`None`) or continues a conversation with a user message and
    /// returns its history including that message. An unknown id is
    /// `NotFound`, a conversation with a run is `AssistantBusy`, one that
    /// holds [`MAX_MESSAGES`] already is `AssistantContextFull`.
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
                if conversation.messages.len() >= MAX_MESSAGES {
                    return Err(AppError::new(ErrorCode::AssistantContextFull));
                }
                conversation.active = true;
                conversation.used = clock;
                conversation.messages.push(ChatMessage::User(message));
                conversation.stamps.push(now_rfc3339());
                id.to_owned()
            }
            None => {
                Self::make_room(&mut table)?;
                let id = Uuid::new_v4().to_string();
                table
                    .map
                    .insert(id.clone(), Conversation::fresh(message, clock));
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

    /// Replaces the history at a consistent point and notes what the run
    /// learned about its tool calls.
    pub(crate) fn commit(&self, id: &str, messages: &[ChatMessage], extras: &RunExtras) {
        if let Some(conversation) = self.lock().map.get_mut(id) {
            conversation.messages = messages.to_vec();
            conversation.stamps.resize_with(messages.len(), now_rfc3339);
            conversation
                .records
                .extend(extras.records.iter().map(|(i, r)| (*i, r.clone())));
            conversation
                .accounts
                .extend(extras.accounts.iter().cloned());
        }
    }

    /// `Some(true)` when the conversation is in memory and has been saved.
    pub(crate) fn saved_state(&self, id: &str) -> Option<bool> {
        self.lock().map.get(id).map(|c| c.in_store)
    }

    /// Puts a conversation read from storage into memory, ready to continue.
    pub(crate) fn insert_loaded(&self, id: &str, loaded: Loaded) -> Result<(), AppError> {
        let mut table = self.lock();
        if table.map.contains_key(id) {
            return Ok(());
        }
        Self::make_room(&mut table)?;
        table.clock += 1;
        let used = table.clock;
        let mut conversation = Conversation {
            saved_messages: loaded.messages.len(),
            saved_rows: loaded.rows,
            messages: loaded.messages,
            stamps: loaded.stamps,
            active: false,
            used,
            in_store: true,
            no_save: false,
            records: BTreeMap::new(),
            accounts: BTreeSet::new(),
        };
        // A reply the app was closed in the middle of.
        conversation.seal();
        table.map.insert(id.to_owned(), conversation);
        Ok(())
    }

    /// Forgets a conversation, run or not. `true` when it was in memory.
    pub(crate) fn remove(&self, id: &str) -> bool {
        self.lock().map.remove(id).is_some()
    }

    /// Forgets a conversation that has no run.
    pub(crate) fn remove_idle(&self, id: &str) {
        let mut table = self.lock();
        if table.map.get(id).is_some_and(|c| !c.active) {
            table.map.remove(id);
        }
    }

    pub(crate) fn clear(&self) {
        self.lock().map.clear();
    }

    /// This conversation is not saved from now on.
    pub(crate) fn stop_saving(&self, id: &str) {
        if let Some(conversation) = self.lock().map.get_mut(id) {
            conversation.no_save = true;
        }
    }

    /// What is not saved yet, plus the closing row of a finished reply.
    /// `None` when there is nothing to write or the conversation is not
    /// saved (any more).
    pub(crate) fn unsaved(&self, id: &str, end: Option<&End>) -> Option<Batch> {
        let table = self.lock();
        let conversation = table.map.get(id)?;
        if conversation.no_save {
            return None;
        }
        let mut rows = Vec::new();
        for (index, message) in conversation
            .messages
            .iter()
            .enumerate()
            .skip(conversation.saved_messages)
        {
            let record = conversation.records.get(&index);
            rows.push(history::message_row(
                message,
                record,
                conversation.stamps.get(index).map_or("", String::as_str),
            ));
        }
        if let Some(end) = end {
            rows.push(history::end_row(end, &now_rfc3339()));
        }
        if rows.is_empty() && conversation.in_store {
            return None;
        }
        let create = (!conversation.in_store)
            .then(|| {
                conversation.messages.iter().find_map(|m| match m {
                    ChatMessage::User(text) => Some(history::title(text)),
                    _ => None,
                })
            })
            .flatten();
        let rows_after = conversation
            .saved_rows
            .saturating_add(u32::try_from(rows.len()).unwrap_or(u32::MAX));
        Some(Batch {
            id: id.to_owned(),
            create,
            from_row: conversation.saved_rows,
            rows,
            accounts: conversation.accounts.iter().cloned().collect(),
            messages_after: conversation.messages.len(),
            rows_after,
        })
    }

    /// The batch reached storage.
    pub(crate) fn mark_saved(&self, batch: &Batch) {
        if let Some(conversation) = self.lock().map.get_mut(&batch.id) {
            conversation.saved_messages = batch.messages_after;
            conversation.saved_rows = batch.rows_after;
            conversation.in_store = true;
            let kept = batch.messages_after;
            conversation.records.retain(|index, _| *index >= kept);
        }
    }

    #[cfg(test)]
    pub(crate) fn is_idle(&self, id: &str) -> bool {
        self.lock().map.get(id).is_some_and(|c| !c.active)
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
