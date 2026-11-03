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
use std::collections::BTreeSet;

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

/// Stable identity of one local archive state (multiple visible messages may share it).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ArchiveKey {
    pub account_id: String,
    source_id: String,
    key: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ArchiveEntry {
    pub key: ArchiveKey,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ArchiveEffects {
    pub account_ids: Vec<String>,
    /// Bounded hints. Consumers invalidate the account as well, so a batch
    /// with many copies does not require an unbounded event payload.
    pub email_ids: Vec<String>,
    pub changed_count: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct ArchiveChange {
    pub processed_count: u32,
    pub touched_keys: BTreeSet<ArchiveKey>,
    pub entries: Vec<ArchiveEntry>,
    pub effects: ArchiveEffects,
}

fn stored(tx: &Transaction<'_>, key: &ArchiveKey) -> Result<Option<String>, AppError> {
    Ok(tx
        .query_row(
            "SELECT archived_at FROM message_archives
         WHERE account_id = ?1 AND mailbox_source_id = ?2 AND archive_key = ?3",
            params![key.account_id, key.source_id, key.key],
            |r| r.get(0),
        )
        .optional()?)
}

fn write_state(
    tx: &Transaction<'_>,
    key: &ArchiveKey,
    value: Option<&str>,
) -> Result<(), AppError> {
    if let Some(value) = value {
        tx.execute(
            "INSERT INTO message_archives (account_id, mailbox_source_id, archive_key, archived_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, mailbox_source_id, archive_key) DO UPDATE SET archived_at = excluded.archived_at",
            params![key.account_id, key.source_id, key.key, value],
        )?;
    } else {
        tx.execute(
            "DELETE FROM message_archives WHERE account_id = ?1 AND mailbox_source_id = ?2 AND archive_key = ?3",
            params![key.account_id, key.source_id, key.key],
        )?;
    }
    Ok(())
}

/// All visible inbox copies sharing the key are affected. Count them, but
/// limit the ids carried by UI invalidation events to 500 across the batch.
fn collect_effects(
    tx: &Transaction<'_>,
    key: &ArchiveKey,
    effects: &mut ArchiveEffects,
) -> Result<(), AppError> {
    let predicate = "FROM messages m
        JOIN accounts a ON a.id = m.account_id AND a.mailbox_source_id = m.mailbox_source_id
        JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id AND f.mailbox_source_id = m.mailbox_source_id
        JOIN folder_sync_cursors c ON c.folder_id = m.folder_id AND c.uid_validity = m.uid_validity
        WHERE m.account_id = ?1 AND m.mailbox_source_id = ?2
          AND CASE WHEN COALESCE(m.message_id_header, '') <> '' THEN 'm:' || m.message_id_header ELSE 'i:' || m.id END = ?3
          AND m.remote_state = 'present' AND f.selectable = 1 AND f.role = 'inbox'
          AND NOT EXISTS (SELECT 1 FROM message_deletions d WHERE d.message_id = m.id)";
    let count: u32 = tx.query_row(
        &format!("SELECT COUNT(*) {predicate}"),
        params![key.account_id, key.source_id, key.key],
        |r| r.get(0),
    )?;
    effects.changed_count = effects.changed_count.saturating_add(count);
    if !effects.account_ids.contains(&key.account_id) {
        effects.account_ids.push(key.account_id.clone());
    }
    let remaining = 500usize.saturating_sub(effects.email_ids.len()) as u32;
    let mut query = tx.prepare(&format!("SELECT m.id {predicate} ORDER BY m.id LIMIT ?4"))?;
    let ids = query.query_map(
        params![key.account_id, key.source_id, key.key, remaining],
        |r| r.get::<_, String>(0),
    )?;
    effects
        .email_ids
        .extend(ids.collect::<Result<Vec<_>, _>>()?);
    Ok(())
}

impl Storage {
    /// Archives the messages locally; archiving one again changes nothing.
    #[cfg(test)]
    pub(crate) fn archive_messages(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.change_archive(message_ids, true).map(|_| ())
    }

    #[cfg(test)]
    pub(crate) fn unarchive_messages(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.change_archive(message_ids, false).map(|_| ())
    }

    /// Validates the complete batch and changes all keys in one transaction.
    pub(crate) fn change_archive(
        &mut self,
        message_ids: &[String],
        archive: bool,
    ) -> Result<ArchiveChange, AppError> {
        let ids: BTreeSet<&str> = message_ids.iter().map(String::as_str).collect();
        if ids.is_empty() || ids.len() > MAX_ARCHIVE_BATCH {
            return Err(AppError::new(ErrorCode::Validation));
        }
        let tx = self.db.transaction()?;
        let mut keys = BTreeSet::new();
        for id in &ids {
            let (account_id, source_id, key) = inbox_key(&tx, id)?;
            keys.insert(ArchiveKey {
                account_id,
                source_id,
                key,
            });
        }
        // Nanoseconds make persisted comparison useful as an additional guard.
        // The application also invalidates receipts on every later operation
        // touching a key, including no-ops and archive/unarchive round trips.
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true);
        let mut change = ArchiveChange {
            processed_count: ids.len() as u32,
            touched_keys: keys,
            entries: Vec::new(),
            effects: ArchiveEffects::default(),
        };
        for key in &change.touched_keys {
            let before = stored(&tx, key)?;
            if before.is_some() == archive {
                continue;
            }
            let after = archive.then(|| now.clone());
            collect_effects(&tx, key, &mut change.effects)?;
            write_state(&tx, key, after.as_deref())?;
            change.entries.push(ArchiveEntry {
                key: key.clone(),
                before,
                after,
            });
        }
        tx.commit()?;
        Ok(change)
    }

    /// Restore only keys changed by this receipt. Any stale state refuses the
    /// whole restore. The application additionally checks its live generation.
    pub(crate) fn restore_archive(
        &mut self,
        entries: &[ArchiveEntry],
    ) -> Result<ArchiveEffects, AppError> {
        let tx = self.db.transaction()?;
        let mut effects = ArchiveEffects::default();
        for entry in entries {
            let current_source: Option<String> = tx
                .query_row(
                    "SELECT mailbox_source_id FROM accounts WHERE id = ?1",
                    [&entry.key.account_id],
                    |r| r.get(0),
                )
                .optional()?;
            if current_source.as_deref() != Some(entry.key.source_id.as_str()) {
                return Err(AppError::new(ErrorCode::NotFound));
            }
            if stored(&tx, &entry.key)? != entry.after {
                return Err(AppError::new(ErrorCode::Conflict));
            }
            collect_effects(&tx, &entry.key, &mut effects)?;
            write_state(&tx, &entry.key, entry.before.as_deref())?;
        }
        tx.commit()?;
        Ok(effects)
    }
}

#[cfg(test)]
mod tests;
