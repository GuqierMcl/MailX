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
    /// The storage row each user message is (or will be) saved at, by its
    /// index in `messages`: a regenerated reply or an edited message is
    /// written over the old rows from there.
    user_rows: BTreeMap<usize, u32>,
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
            user_rows: BTreeMap::from([(0, 0)]),
        }
    }

    /// Notes the storage row of a user message about to go in at `index`:
    /// the rows saved so far, plus one for each message not saved yet.
    fn note_user_row(&mut self, index: usize) {
        let row = u32::try_from(index.saturating_sub(self.saved_messages))
            .ok()
            .and_then(|unsaved| self.saved_rows.checked_add(unsaved));
        if let Some(row) = row {
            self.user_rows.insert(index, row);
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
                reasoning: None,
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
    /// What a regeneration took away, put back if the new run never starts:
    /// the old reply must not be lost because asking again failed.
    restore: Option<Restore>,
    /// Went on with an earlier reply: there is no new user message to take back.
    continued: bool,
}

struct Restore {
    /// How long the history was cut to; anything the run added after it goes.
    keep: usize,
    messages: Vec<ChatMessage>,
    stamps: Vec<String>,
    user_rows: BTreeMap<usize, u32>,
    saved_messages: usize,
    saved_rows: u32,
    no_save: bool,
}

/// What a run in place of earlier history answers.
enum Redo {
    /// The last user message again, in place of its reply.
    Answer,
    /// The `index`-th user message (from 0), reworded: it and everything
    /// after it are replaced.
    Edit { index: usize, message: String },
}

impl ActiveConversation {
    /// The run never started: takes the user message back out (a conversation
    /// that held only that message disappears) and frees the conversation.
    pub(crate) fn rollback(mut self) {
        self.released = true;
        let restore = self.restore.take();
        let continued = self.continued;
        let mut table = self.store.lock();
        if let Some(conversation) = table.map.get_mut(&self.id) {
            match restore {
                Some(restore) => {
                    conversation.messages.truncate(restore.keep);
                    conversation.stamps.truncate(restore.keep);
                    conversation.messages.extend(restore.messages);
                    conversation.stamps.extend(restore.stamps);
                    conversation.user_rows.extend(restore.user_rows);
                    conversation.saved_messages = restore.saved_messages;
                    conversation.saved_rows = restore.saved_rows;
                    conversation.no_save = restore.no_save;
                }
                None if continued => {}
                None => {
                    conversation.messages.pop();
                    conversation.stamps.pop();
                }
            }
            conversation.active = false;
            conversation.seal();
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
                let index = conversation.messages.len();
                conversation.note_user_row(index);
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
                restore: None,
                continued: false,
            },
            history,
        ))
    }

    /// Goes on with the conversation's last reply: the history stays as it
    /// ends (the last reply, stopped or cut off), and the model is asked to
    /// continue it. `NotFound` for an unknown id, `AssistantBusy` while a run
    /// holds it, `Validation` when there is no reply to continue.
    pub(crate) fn begin_continue(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<(ActiveConversation, Vec<ChatMessage>), AppError> {
        let mut table = self.lock();
        table.clock += 1;
        let clock = table.clock;
        let conversation = table
            .map
            .get_mut(id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        if conversation.active {
            return Err(AppError::new(ErrorCode::AssistantBusy));
        }
        if !matches!(
            conversation.messages.last(),
            Some(ChatMessage::Assistant { .. })
        ) {
            return Err(AppError::validation("conversationId"));
        }
        conversation.active = true;
        conversation.used = clock;
        let history = conversation.messages.clone();
        drop(table);
        Ok((
            ActiveConversation {
                store: self.clone(),
                id: id.to_owned(),
                released: false,
                restore: None,
                continued: true,
            },
            history,
        ))
    }

    /// Takes back the last reply so the last user message can be answered
    /// again, and returns the history ending with that message. Only a reply
    /// that changed nothing can go: re-running one that archived, drafted or
    /// sent would do it twice (`Conflict`). `NotFound` for an unknown id,
    /// `AssistantBusy` while a run holds it, `Validation` without a user
    /// message.
    pub(crate) fn begin_regenerate(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<(ActiveConversation, Vec<ChatMessage>), AppError> {
        self.begin_redo(id, Redo::Answer)
    }

    /// Rewords the `index`-th user message (from 0): it and everything after
    /// it are replaced by `message`, and the history ending with it is
    /// returned. As for [`Self::begin_regenerate`], nothing after it may have
    /// changed mail (`Conflict`); an index past the last user message is
    /// `Validation`.
    pub(crate) fn begin_edit(
        self: &Arc<Self>,
        id: &str,
        index: usize,
        message: String,
    ) -> Result<(ActiveConversation, Vec<ChatMessage>), AppError> {
        self.begin_redo(id, Redo::Edit { index, message })
    }

    fn begin_redo(
        self: &Arc<Self>,
        id: &str,
        redo: Redo,
    ) -> Result<(ActiveConversation, Vec<ChatMessage>), AppError> {
        let mut table = self.lock();
        table.clock += 1;
        let clock = table.clock;
        let conversation = table
            .map
            .get_mut(id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        if conversation.active {
            return Err(AppError::new(ErrorCode::AssistantBusy));
        }
        let mut users = conversation
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| matches!(message, ChatMessage::User(_)))
            .map(|(at, _)| at);
        let (user, keep, reworded) = match redo {
            Redo::Answer => {
                let at = users
                    .next_back()
                    .ok_or_else(|| AppError::validation("conversationId"))?;
                (at, at + 1, None)
            }
            Redo::Edit { index, message } => {
                let at = users
                    .nth(index)
                    .ok_or_else(|| AppError::validation("edit"))?;
                (at, at, Some(message))
            }
        };
        // Everything that goes must have changed nothing.
        let changed = conversation.messages[user + 1..]
            .iter()
            .any(|message| match message {
                ChatMessage::Assistant { tool_calls, .. } => tool_calls
                    .iter()
                    .any(|call| !super::policy::is_read(&call.name)),
                _ => false,
            });
        if changed {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let restore = Restore {
            keep,
            messages: conversation.messages[keep..].to_vec(),
            stamps: conversation.stamps[keep..].to_vec(),
            user_rows: conversation.user_rows.split_off(&keep),
            saved_messages: conversation.saved_messages,
            saved_rows: conversation.saved_rows,
            no_save: conversation.no_save,
        };
        conversation.messages.truncate(keep);
        conversation.stamps.truncate(keep);
        conversation.records.retain(|index, _| *index < keep);
        if conversation.saved_messages > keep {
            // What goes is in storage: the next save writes over it, from
            // the row the first message to go was saved at. Without that row
            // the conversation is not saved again rather than saved wrong.
            conversation.saved_messages = keep;
            // Answering again keeps the user message and writes after it;
            // an edit writes over the message itself.
            let row = if keep > user {
                conversation
                    .user_rows
                    .get(&user)
                    .and_then(|row| row.checked_add(1))
            } else {
                restore.user_rows.get(&user).copied()
            };
            match row {
                Some(row) => conversation.saved_rows = row,
                None => conversation.no_save = true,
            }
        }
        if let Some(message) = reworded {
            conversation.note_user_row(keep);
            conversation.messages.push(ChatMessage::User(message));
            conversation.stamps.push(now_rfc3339());
        }
        conversation.active = true;
        conversation.used = clock;
        let history = conversation.messages.clone();
        drop(table);
        Ok((
            ActiveConversation {
                store: self.clone(),
                id: id.to_owned(),
                released: false,
                restore: Some(restore),
                continued: false,
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
            user_rows: loaded.user_rows,
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
