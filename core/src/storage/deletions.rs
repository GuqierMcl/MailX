//! Deletions waiting to be carried out on the server.
//!
//! Asking to delete hides the messages at once and schedules the server work
//! a few seconds later, so the user can still take it back. The account's
//! write-back worker claims due rows (after which undo is refused), moves them
//! into Trash or expunges them, and then turns each message into a `deleted`
//! tombstone. A deletion that cannot be done gives the message back.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use super::{Storage, search_index};
use crate::error::{AppError, ErrorCode};

/// The most messages one request may delete.
pub(crate) const MAX_DELETION_BATCH: usize = 500;

/// What a delete request turned into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledDeletion {
    pub(crate) batch_id: String,
    pub(crate) due_at: DateTime<Utc>,
    /// Whether any of the messages is expunged rather than moved to Trash.
    pub(crate) permanent: bool,
    /// Accounts whose worker has new work, without repeats.
    pub(crate) account_ids: Vec<String>,
}

/// A due deletion as the write-back worker needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingDeletion {
    pub(crate) message_id: String,
    pub(crate) folder_id: String,
    pub(crate) folder_remote_name: String,
    pub(crate) uid_validity: u32,
    pub(crate) uid: u32,
    /// The Trash folder to move into; `None` expunges for good.
    pub(crate) target_remote_name: Option<String>,
    pub(crate) attempts: u32,
}

/// Rows still claimed by the worker after this many failed rounds are given
/// back to the user instead of staying hidden for ever.
pub(crate) const MAX_DELETION_ATTEMPTS: u32 = 5;

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|_| AppError::new(ErrorCode::Internal))
}

/// Puts back the unread number a hidden unread message took away.
fn adjust_unread(tx: &Transaction<'_>, message_id: &str, delta: i64) -> Result<(), AppError> {
    tx.execute(
        "UPDATE folders SET unread_count = MAX(0, unread_count + ?2)
         WHERE id = (SELECT folder_id FROM messages WHERE id = ?1 AND is_read = 0)
           AND synced_at <> ''",
        params![message_id, delta],
    )?;
    Ok(())
}

impl Storage {
    /// Hides the messages and schedules their deletion for `due_at`.
    ///
    /// A message outside Trash goes to the account's Trash folder. One already
    /// in Trash, or in an account without one, is expunged for good, which is
    /// refused unless the caller says the user confirmed it.
    pub(crate) fn schedule_deletion(
        &mut self,
        message_ids: &[String],
        confirmed_permanent: bool,
        now: DateTime<Utc>,
        delay: Duration,
    ) -> Result<ScheduledDeletion, AppError> {
        let mut ids: Vec<&str> = Vec::with_capacity(message_ids.len());
        for id in message_ids {
            if !ids.contains(&id.as_str()) {
                ids.push(id);
            }
        }
        if ids.is_empty() || ids.len() > MAX_DELETION_BATCH {
            return Err(AppError::new(ErrorCode::Validation));
        }
        let tx = self.db.transaction()?;
        let batch_id = Uuid::new_v4().to_string();
        let due_at = now + delay;
        let mut permanent_any = false;
        let mut account_ids: Vec<String> = Vec::new();
        for id in ids {
            // Only a message the user can see, from the server generation the
            // cursor still trusts, can be deleted; one already waiting for
            // deletion is hidden and so reads as missing too.
            let row: Option<(String, String, String, String, u32, u32)> = tx
                .query_row(
                    "SELECT m.account_id, m.mailbox_source_id, m.folder_id, f.role, m.uid_validity, m.uid
                     FROM messages m
                     JOIN accounts a ON a.id = m.account_id AND a.mailbox_source_id = m.mailbox_source_id
                     JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id
                          AND f.mailbox_source_id = m.mailbox_source_id
                     JOIN folder_sync_cursors c ON c.folder_id = m.folder_id AND c.uid_validity = m.uid_validity
                     WHERE m.id = ?1 AND m.remote_state = 'present' AND f.selectable = 1
                       AND NOT EXISTS (SELECT 1 FROM message_deletions d WHERE d.message_id = m.id)",
                    [id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .optional()?;
            let Some((account_id, source_id, folder_id, role, uid_validity, uid)) = row else {
                return Err(AppError::new(ErrorCode::NotFound));
            };
            let trash: Option<String> = if role == "trash" {
                None
            } else {
                tx.query_row(
                    "SELECT id FROM folders
                     WHERE account_id = ?1 AND mailbox_source_id = ?2 AND role = 'trash'
                       AND selectable = 1
                     ORDER BY remote_name LIMIT 1",
                    params![account_id, source_id],
                    |row| row.get(0),
                )
                .optional()?
            };
            let permanent = trash.is_none();
            if permanent && !confirmed_permanent {
                return Err(AppError::new(ErrorCode::ConfirmationRequired));
            }
            permanent_any |= permanent;
            tx.execute(
                "INSERT INTO message_deletions (
                    message_id, batch_id, account_id, mailbox_source_id, folder_id,
                    uid_validity, uid, target_folder_id, permanent, due_at, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    batch_id,
                    account_id,
                    source_id,
                    folder_id,
                    uid_validity,
                    uid,
                    trash,
                    permanent,
                    timestamp(due_at),
                    timestamp(now),
                ],
            )?;
            adjust_unread(&tx, id, -1)?;
            if !account_ids.contains(&account_id) {
                account_ids.push(account_id);
            }
        }
        tx.commit()?;
        Ok(ScheduledDeletion {
            batch_id,
            due_at,
            permanent: permanent_any,
            account_ids,
        })
    }

    /// Takes back a deletion the worker has not started, showing its messages
    /// again. Returns the accounts touched. A batch that is already running is
    /// refused; one that is gone (finished or given back) reads as missing.
    pub(crate) fn cancel_deletion(&mut self, batch_id: &str) -> Result<Vec<String>, AppError> {
        let tx = self.db.transaction()?;
        let rows: Vec<(String, String, bool)> = {
            let mut statement = tx.prepare(
                "SELECT message_id, account_id, started FROM message_deletions WHERE batch_id = ?1",
            )?;
            statement
                .query_map([batch_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect::<Result<_, _>>()?
        };
        if rows.is_empty() {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        // Undo is all or nothing: half a batch coming back would be confusing.
        if rows.iter().any(|(_, _, started)| *started) {
            return Err(AppError::new(ErrorCode::AlreadyDeleting));
        }
        let mut account_ids: Vec<String> = Vec::new();
        for (message_id, account_id, _) in rows {
            adjust_unread(&tx, &message_id, 1)?;
            if !account_ids.contains(&account_id) {
                account_ids.push(account_id);
            }
        }
        tx.execute(
            "DELETE FROM message_deletions WHERE batch_id = ?1",
            [batch_id],
        )?;
        tx.commit()?;
        Ok(account_ids)
    }

    /// Claims the account's deletions that are due, so undo can no longer race
    /// the server. Rows the worker claimed before (a failed earlier round) come
    /// back too. Rows whose folder, Trash or server generation changed since
    /// they were scheduled can no longer be carried out and are given back;
    /// their count is returned alongside.
    pub(crate) fn claim_due_deletions(
        &mut self,
        account_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(Vec<PendingDeletion>, u32), AppError> {
        let tx = self.db.transaction()?;
        type Row = (
            String,
            String,
            Option<String>,
            u32,
            u32,
            bool,
            Option<String>,
            u32,
            bool,
        );
        let rows: Vec<Row> = {
            let mut statement = tx.prepare(
                "SELECT d.message_id, d.folder_id, f.remote_name, d.uid_validity, d.uid, d.permanent,
                        t.remote_name, d.attempts,
                        (a.mailbox_source_id = d.mailbox_source_id AND f.selectable = 1
                         AND c.uid_validity = d.uid_validity
                         AND (d.permanent = 1 OR (t.id IS NOT NULL AND t.selectable = 1))) AS usable
                 FROM message_deletions d
                 JOIN accounts a ON a.id = d.account_id
                 LEFT JOIN folders f ON f.id = d.folder_id
                 LEFT JOIN folder_sync_cursors c ON c.folder_id = d.folder_id
                 LEFT JOIN folders t ON t.id = d.target_folder_id
                 WHERE d.account_id = ?1 AND (d.started = 1 OR d.due_at <= ?2)
                 ORDER BY d.due_at, d.folder_id, d.uid",
            )?;
            statement
                .query_map(params![account_id, timestamp(now)], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get::<_, Option<bool>>(8)?.unwrap_or(false),
                    ))
                })?
                .collect::<Result<_, _>>()?
        };
        let mut due = Vec::with_capacity(rows.len());
        let mut abandoned = 0;
        for (
            message_id,
            folder_id,
            folder_name,
            uid_validity,
            uid,
            permanent,
            target,
            attempts,
            usable,
        ) in rows
        {
            let (true, Some(folder_remote_name)) = (usable, folder_name) else {
                adjust_unread(&tx, &message_id, 1)?;
                tx.execute(
                    "DELETE FROM message_deletions WHERE message_id = ?1",
                    [&message_id],
                )?;
                abandoned += 1;
                continue;
            };
            tx.execute(
                "UPDATE message_deletions SET started = 1 WHERE message_id = ?1",
                [&message_id],
            )?;
            due.push(PendingDeletion {
                message_id,
                folder_id,
                folder_remote_name,
                uid_validity,
                uid,
                target_remote_name: if permanent { None } else { target },
                attempts,
            });
        }
        tx.commit()?;
        Ok((due, abandoned))
    }

    /// When the account's next not-yet-started deletion falls due, if any.
    pub(crate) fn next_deletion_due(
        &self,
        account_id: &str,
    ) -> Result<Option<DateTime<Utc>>, AppError> {
        let next: Option<String> = self.db.query_row(
            "SELECT MIN(due_at) FROM message_deletions WHERE account_id = ?1 AND started = 0",
            [account_id],
            |row| row.get(0),
        )?;
        next.as_deref().map(parse_timestamp).transpose()
    }

    /// Records that the server no longer has these messages where they were:
    /// each becomes a `deleted` tombstone, leaves the search index, and drops
    /// any flag change still waiting for it.
    pub(crate) fn finish_deletions(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        let tx = self.db.transaction()?;
        let stamp = timestamp(Utc::now());
        for id in message_ids {
            tx.execute(
                "UPDATE messages SET remote_state = 'deleted', updated_at = ?2
                 WHERE id = ?1 AND remote_state = 'present'",
                params![id, stamp],
            )?;
            search_index::refresh_message(&tx, id)?;
            tx.execute("DELETE FROM flag_changes WHERE message_id = ?1", [id])?;
            tx.execute("DELETE FROM message_deletions WHERE message_id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Notes a failed try. Rows that have failed too often are given back to
    /// the user; the number given back is returned.
    pub(crate) fn deletion_attempt_failed(
        &mut self,
        message_ids: &[String],
    ) -> Result<u32, AppError> {
        let tx = self.db.transaction()?;
        let mut given_back = 0;
        for id in message_ids {
            let attempts: Option<u32> = tx
                .query_row(
                    "UPDATE message_deletions SET attempts = attempts + 1
                     WHERE message_id = ?1 RETURNING attempts",
                    [id],
                    |row| row.get(0),
                )
                .optional()?;
            if attempts.is_some_and(|attempts| attempts >= MAX_DELETION_ATTEMPTS) {
                adjust_unread(&tx, id, 1)?;
                tx.execute("DELETE FROM message_deletions WHERE message_id = ?1", [id])?;
                given_back += 1;
            }
        }
        tx.commit()?;
        Ok(given_back)
    }

    /// Gives deletions back at once, for work the server cannot do at all.
    pub(crate) fn abandon_deletions(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        let tx = self.db.transaction()?;
        for id in message_ids {
            // Only a row still waiting took the unread number away.
            if tx.execute("DELETE FROM message_deletions WHERE message_id = ?1", [id])? > 0 {
                adjust_unread(&tx, id, 1)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
