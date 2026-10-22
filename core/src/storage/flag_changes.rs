//! Read and starred changes the user made, kept locally and written back to the
//! server later. The local copy is authoritative for as long as a change is
//! waiting, so a sync round that still sees the old server value never makes a
//! toggle snap back.
use rusqlite::{OptionalExtension, params};

use super::Storage;
use crate::{AppError, ErrorCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Flag {
    Seen,
    Flagged,
}

impl Flag {
    fn column(self) -> &'static str {
        match self {
            Self::Seen => "is_read",
            Self::Flagged => "is_flagged",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Seen => "seen",
            Self::Flagged => "flagged",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "seen" => Some(Self::Seen),
            "flagged" => Some(Self::Flagged),
            _ => None,
        }
    }

    /// The IMAP flag as it appears on the wire.
    pub(crate) fn imap(self) -> &'static str {
        match self {
            Self::Seen => "\\Seen",
            Self::Flagged => "\\Flagged",
        }
    }
}

/// Where a message lives on the server, enough to address it in a new session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingChange {
    pub message_id: String,
    pub account_id: String,
    pub mailbox_source_id: String,
    pub folder_id: String,
    pub remote_name: String,
    pub uid_validity: u32,
    pub uid: u32,
    pub flag: Flag,
    pub wanted: bool,
    pub attempts: u32,
    /// Whether the server said (or is known) to store this flag in that folder.
    pub folder_can_flag: Option<bool>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// What a local change turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalChange {
    /// The message already had this value; nothing to record.
    Unchanged,
    /// Stored as pending; a worker will write it back.
    Queued,
    /// Stored for good on this device only: the folder cannot keep this flag.
    LocalOnly,
}

impl Storage {
    /// Sets a flag on a visible message and records the wish to write it back.
    /// A change that turns the flag back to what the server last confirmed
    /// cancels the earlier one instead of queueing a pointless round trip.
    pub(crate) fn set_message_flag(
        &mut self,
        message_id: &str,
        flag: Flag,
        wanted: bool,
    ) -> Result<LocalChange, AppError> {
        let tx = self.db.transaction()?;
        // A message the user can no longer see (deleted, or from a replaced
        // server generation) is refused rather than silently edited.
        let row: Option<(bool, Option<bool>)> = tx
            .query_row(
                &format!(
                    "SELECT m.{column}, f.can_flag
                     FROM messages m
                     JOIN accounts a ON a.id = m.account_id AND a.mailbox_source_id = m.mailbox_source_id
                     JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id
                          AND f.mailbox_source_id = m.mailbox_source_id
                     JOIN folder_sync_cursors c ON c.folder_id = m.folder_id AND c.uid_validity = m.uid_validity
                     WHERE m.id = ?1 AND m.remote_state = 'present' AND f.selectable = 1",
                    column = flag.column()
                ),
                [message_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current, folder_can_flag)) = row else {
            return Err(AppError::new(ErrorCode::NotFound));
        };
        if current == wanted {
            return Ok(LocalChange::Unchanged);
        }
        let timestamp = now();
        tx.execute(
            &format!(
                "UPDATE messages SET {column} = ?2, updated_at = ?3 WHERE id = ?1",
                column = flag.column()
            ),
            params![message_id, wanted, timestamp],
        )?;
        // The starred flag on a folder known not to keep it is final right away.
        let local_only = flag == Flag::Flagged && folder_can_flag == Some(false);
        tx.execute(
            "INSERT INTO flag_changes (message_id, flag, wanted, state, attempts, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)
             ON CONFLICT (message_id, flag) DO UPDATE SET
                wanted = excluded.wanted, state = excluded.state, attempts = 0,
                updated_at = excluded.updated_at",
            params![
                message_id,
                flag.name(),
                wanted,
                if local_only { "local" } else { "pending" },
                timestamp
            ],
        )?;
        tx.commit()?;
        Ok(if local_only {
            LocalChange::LocalOnly
        } else {
            LocalChange::Queued
        })
    }

    /// Changes still waiting for the server, oldest first, for one account.
    pub(crate) fn pending_changes(
        &self,
        account_id: &str,
        limit: usize,
    ) -> Result<Vec<PendingChange>, AppError> {
        let mut statement = self.db.prepare(
            "SELECT m.id, m.account_id, m.mailbox_source_id, m.folder_id, f.remote_name,
                    m.uid_validity, m.uid, c.flag, c.wanted, c.attempts, f.can_flag
             FROM flag_changes c
             JOIN messages m ON m.id = c.message_id
             JOIN accounts a ON a.id = m.account_id AND a.mailbox_source_id = m.mailbox_source_id
             JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id
                  AND f.mailbox_source_id = m.mailbox_source_id
             JOIN folder_sync_cursors s ON s.folder_id = m.folder_id AND s.uid_validity = m.uid_validity
             WHERE m.account_id = ?1 AND c.state = 'pending' AND m.remote_state = 'present'
             ORDER BY c.updated_at, c.message_id
             LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![account_id, limit as i64], |row| {
                let flag: String = row.get(7)?;
                Ok((
                    PendingChange {
                        message_id: row.get(0)?,
                        account_id: row.get(1)?,
                        mailbox_source_id: row.get(2)?,
                        folder_id: row.get(3)?,
                        remote_name: row.get(4)?,
                        uid_validity: row.get(5)?,
                        uid: row.get(6)?,
                        flag: Flag::Seen,
                        wanted: row.get(8)?,
                        attempts: row.get(9)?,
                        folder_can_flag: row.get(10)?,
                    },
                    flag,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(change, flag)| {
                Flag::parse(&flag)
                    .map(|flag| PendingChange { flag, ..change })
                    .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))
            })
            .collect()
    }

    /// The server confirmed the change. It is only removed while it still says
    /// what was sent: a newer toggle made meanwhile stays queued.
    pub(crate) fn finish_change(
        &mut self,
        message_id: &str,
        flag: Flag,
        sent: bool,
    ) -> Result<bool, AppError> {
        let removed = self.db.execute(
            "DELETE FROM flag_changes WHERE message_id = ?1 AND flag = ?2 AND wanted = ?3 AND state = 'pending'",
            params![message_id, flag.name(), sent],
        )?;
        Ok(removed > 0)
    }

    /// A transient failure: keep the change and count the attempt.
    pub(crate) fn retry_change(&mut self, message_id: &str, flag: Flag) -> Result<(), AppError> {
        self.db.execute(
            "UPDATE flag_changes SET attempts = attempts + 1, updated_at = ?3
             WHERE message_id = ?1 AND flag = ?2 AND state = 'pending'",
            params![message_id, flag.name(), now()],
        )?;
        Ok(())
    }

    /// The server cannot store this flag in this folder: remember that for the
    /// folder and keep every starred change there on this device only. Returns
    /// how many changes moved from waiting to local-only.
    pub(crate) fn mark_folder_unflaggable(&mut self, folder_id: &str) -> Result<usize, AppError> {
        let tx = self.db.transaction()?;
        tx.execute("UPDATE folders SET can_flag = 0 WHERE id = ?1", [folder_id])?;
        let moved = tx.execute(
            "UPDATE flag_changes SET state = 'local', updated_at = ?2
             WHERE flag = 'flagged' AND state = 'pending'
               AND message_id IN (SELECT id FROM messages WHERE folder_id = ?1)",
            params![folder_id, now()],
        )?;
        tx.commit()?;
        Ok(moved)
    }

    pub(crate) fn mark_folder_flaggable(&mut self, folder_id: &str) -> Result<(), AppError> {
        self.db.execute(
            "UPDATE folders SET can_flag = 1 WHERE id = ?1 AND can_flag IS NOT 1",
            [folder_id],
        )?;
        Ok(())
    }
}
