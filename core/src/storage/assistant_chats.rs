//! Saved assistant conversations: a header, the accounts its tool calls
//! touched, and its messages in order. The message content is opaque JSON
//! here; `assistant::history` owns its shape.
use rusqlite::{OptionalExtension, params};

use super::Storage;
use crate::{AppError, ErrorCode};

/// Conversations kept, newest first; older ones are deleted by the write that
/// pushes the count over.
pub(crate) const KEEP_CONVERSATIONS: usize = 100;

pub(crate) struct NewRow {
    pub role: &'static str,
    pub content: String,
    pub created_at: String,
}

/// One write: rows from `from_seq` on replace whatever is stored there.
pub(crate) struct Append<'a> {
    pub id: &'a str,
    /// The conversation's first save, with its title. Without it a missing
    /// conversation is left missing (it was deleted in the meantime).
    pub create: Option<&'a str>,
    pub from_seq: u32,
    pub rows: &'a [NewRow],
    pub accounts: &'a [String],
    pub now: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    pub message_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredRow {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

pub(crate) struct StoredConversation {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub rows: Vec<StoredRow>,
}

impl Storage {
    /// `false` when the conversation does not exist and was not created.
    pub(crate) fn assistant_chat_append(&mut self, write: &Append<'_>) -> Result<bool, AppError> {
        let tx = self.db.transaction()?;
        if let Some(title) = write.create {
            tx.execute(
                "INSERT OR IGNORE INTO assistant_conversations(id, title, created_at, updated_at, message_count)
                 VALUES (?1, ?2, ?3, ?3, 0)",
                params![write.id, title, write.now],
            )?;
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_conversations WHERE id = ?1)",
            [write.id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM assistant_messages WHERE conversation_id = ?1 AND seq >= ?2",
            params![write.id, write.from_seq],
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO assistant_messages(conversation_id, seq, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (offset, row) in write.rows.iter().enumerate() {
                let seq = u32::try_from(offset)
                    .ok()
                    .and_then(|offset| write.from_seq.checked_add(offset))
                    .ok_or_else(|| AppError::new(ErrorCode::Internal))?;
                insert.execute(params![
                    write.id,
                    seq,
                    row.role,
                    row.content,
                    row.created_at
                ])?;
            }
        }
        for account in write.accounts {
            tx.execute(
                "INSERT OR IGNORE INTO assistant_conversation_accounts(conversation_id, account_id)
                 VALUES (?1, ?2)",
                params![write.id, account],
            )?;
        }
        tx.execute(
            "UPDATE assistant_conversations SET updated_at = ?2,
               message_count = (SELECT count(*) FROM assistant_messages
                                WHERE conversation_id = ?1 AND role = 'user')
             WHERE id = ?1",
            params![write.id, write.now],
        )?;
        tx.execute(
            "DELETE FROM assistant_conversations WHERE id NOT IN (
               SELECT id FROM assistant_conversations ORDER BY updated_at DESC, id DESC LIMIT ?1)",
            [i64::try_from(KEEP_CONVERSATIONS).unwrap_or(i64::MAX)],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Newest first. `after` is the `(updated_at, id)` of the last item of the
    /// previous page.
    pub(crate) fn assistant_chat_list(
        &self,
        limit: usize,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<StoredSummary>, AppError> {
        let (after_at, after_id) = after.unzip();
        let mut statement = self.db.prepare(
            "SELECT id, title, updated_at, message_count FROM assistant_conversations
             WHERE ?1 IS NULL OR updated_at < ?1 OR (updated_at = ?1 AND id < ?2)
             ORDER BY updated_at DESC, id DESC LIMIT ?3",
        )?;
        Ok(statement
            .query_map(
                params![after_at, after_id, i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| {
                    Ok(StoredSummary {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        updated_at: row.get(2)?,
                        message_count: row.get(3)?,
                    })
                },
            )?
            .collect::<Result<_, _>>()?)
    }

    pub(crate) fn assistant_chat_exists(&self, id: &str) -> Result<bool, AppError> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_conversations WHERE id = ?1)",
            [id],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn assistant_chat_get(
        &self,
        id: &str,
    ) -> Result<Option<StoredConversation>, AppError> {
        let header = self
            .db
            .query_row(
                "SELECT title, created_at, updated_at FROM assistant_conversations WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((title, created_at, updated_at)) = header else {
            return Ok(None);
        };
        let mut statement = self.db.prepare(
            "SELECT role, content, created_at FROM assistant_messages
             WHERE conversation_id = ?1 ORDER BY seq",
        )?;
        let rows = statement
            .query_map([id], |row| {
                Ok(StoredRow {
                    role: row.get(0)?,
                    content: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(Some(StoredConversation {
            id: id.to_owned(),
            title,
            created_at,
            updated_at,
            rows,
        }))
    }

    /// `true` when a conversation was deleted.
    pub(crate) fn assistant_chat_delete(&mut self, id: &str) -> Result<bool, AppError> {
        Ok(self
            .db
            .execute("DELETE FROM assistant_conversations WHERE id = ?1", [id])?
            > 0)
    }

    pub(crate) fn assistant_chat_clear(&mut self) -> Result<(), AppError> {
        self.db.execute("DELETE FROM assistant_conversations", [])?;
        Ok(())
    }
}
