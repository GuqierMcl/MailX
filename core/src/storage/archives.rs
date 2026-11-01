//! Local-only archive of inbox messages.
//!
//! Archiving never talks to the server: the message stays in INBOX there and
//! MailX only stops listing it in the inbox views. The state lives in
//! `message_archives`, keyed by account, mailbox source and the message's
//! `Message-ID` header, so it survives a UIDVALIDITY reset that rebuilds the
//! message rows. Mail without that header is keyed by its local id and comes
//! back to the inbox after such a reset. Two inbox copies sharing a
//! `Message-ID` are archived together, and a message that returns to an inbox
//! with an archived `Message-ID` shows up archived again.
//!
//! A later "also move to the server's Archive folder" strategy can be added
//! per account on top of this without changing the list or command shapes.

use chrono::{SecondsFormat, Utc};
use rusqlite::{OptionalExtension, Transaction, params};

use super::Storage;
use crate::error::{AppError, ErrorCode};

/// The most messages one request may archive or unarchive.
pub(crate) const MAX_ARCHIVE_BATCH: usize = 500;

/// Account, mailbox source and archive key of a visible inbox message.
fn inbox_key(tx: &Transaction<'_>, message_id: &str) -> Result<(String, String, String), AppError> {
    tx.query_row(
        "SELECT m.account_id, m.mailbox_source_id,
                CASE WHEN COALESCE(m.message_id_header, '') <> ''
                     THEN 'm:' || m.message_id_header ELSE 'i:' || m.id END
         FROM messages m
         JOIN accounts a ON a.id = m.account_id AND a.mailbox_source_id = m.mailbox_source_id
         JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id
              AND f.mailbox_source_id = m.mailbox_source_id
         JOIN folder_sync_cursors c ON c.folder_id = m.folder_id AND c.uid_validity = m.uid_validity
         WHERE m.id = ?1 AND m.remote_state = 'present' AND f.selectable = 1
           AND f.role = 'inbox'
           AND NOT EXISTS (SELECT 1 FROM message_deletions d WHERE d.message_id = m.id)",
        [message_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()?
    .ok_or_else(|| AppError::new(ErrorCode::NotFound))
}

impl Storage {
    /// Archives the messages locally; archiving one again changes nothing.
    ///
    /// Every id must be a visible message of an inbox folder, or the whole
    /// batch is refused with `NotFound` and nothing changes.
    pub(crate) fn archive_messages(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.change_archive(message_ids, true)
    }

    /// Takes the messages back into the inbox; the same rules as archiving.
    pub(crate) fn unarchive_messages(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.change_archive(message_ids, false)
    }

    fn change_archive(&mut self, message_ids: &[String], archive: bool) -> Result<(), AppError> {
        let mut ids: Vec<&str> = Vec::with_capacity(message_ids.len());
        for id in message_ids {
            if !ids.contains(&id.as_str()) {
                ids.push(id);
            }
        }
        if ids.is_empty() || ids.len() > MAX_ARCHIVE_BATCH {
            return Err(AppError::new(ErrorCode::Validation));
        }
        let tx = self.db.transaction()?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        for id in ids {
            let (account_id, source_id, key) = inbox_key(&tx, id)?;
            if archive {
                tx.execute(
                    "INSERT OR IGNORE INTO message_archives
                        (account_id, mailbox_source_id, archive_key, archived_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![account_id, source_id, key, now],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM message_archives
                     WHERE account_id = ?1 AND mailbox_source_id = ?2 AND archive_key = ?3",
                    params![account_id, source_id, key],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
