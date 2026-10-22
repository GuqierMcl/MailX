//! Cached message headers and committed per-folder synchronization progress.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Storage, decode, encode};
use crate::{
    AppError, ErrorCode,
    mail::{
        AttachmentSummary, BodyState, Coverage, EmailBody, EmailDetail, EmailListQuery, EmailPage,
        EmailSummary, MailAddress,
    },
    sync::headers::MessageRecord,
};

/// Identity captured before network I/O; every write checks it again in its transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyncTarget {
    pub account_id: String,
    pub mailbox_source_id: String,
    pub folder_id: String,
    pub uid_validity: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderSyncCursor {
    pub uid_validity: u32,
    pub window_start: String,
    pub window_capped: bool,
    pub backfill_done: bool,
    pub highest_uid: u32,
    pub updated_at: String,
    /// Set when this call started a new generation (first sync or UIDVALIDITY
    /// change), which hides previously visible messages. Not stored.
    pub created: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageCursor {
    pub r: String,
    pub i: String,
}

/// AccountService validates and normalizes all external values before SQL sees them.
pub(crate) struct ValidatedEmailQuery {
    pub query: EmailListQuery,
    pub cursor: Option<PageCursor>,
    pub limit: u32,
}

/// Flags in a stable order so the same server state always encodes the same
/// way; servers may list flags in any order between FETCH responses.
fn encode_flags(flags: &[String]) -> Result<String, AppError> {
    let mut sorted = flags.to_vec();
    sorted.sort_by_key(|flag| flag.to_ascii_lowercase());
    sorted.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    encode(&sorted)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn current_folder(
    db: &Connection,
    account_id: &str,
    source: &str,
    folder_id: &str,
) -> Result<bool, AppError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders f JOIN accounts a ON a.id = f.account_id
         WHERE f.id = ?1 AND f.account_id = ?2 AND f.mailbox_source_id = ?3
         AND a.mailbox_source_id = ?3 AND f.selectable = 1)",
        params![folder_id, account_id, source],
        |row| row.get(0),
    )?)
}

fn current_target(db: &Connection, target: &SyncTarget) -> Result<bool, AppError> {
    if !current_folder(
        db,
        &target.account_id,
        &target.mailbox_source_id,
        &target.folder_id,
    )? {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM folder_sync_cursors WHERE folder_id = ?1 AND uid_validity = ?2)",
        params![target.folder_id, target.uid_validity],
        |row| row.get(0),
    )?)
}

fn read_cursor(db: &Connection, folder_id: &str) -> Result<Option<FolderSyncCursor>, AppError> {
    Ok(db.query_row(
        "SELECT uid_validity, window_start, window_capped, backfill_done, highest_uid, updated_at
         FROM folder_sync_cursors WHERE folder_id = ?1",
        [folder_id],
        |row| Ok(FolderSyncCursor {
            uid_validity: row.get(0)?, window_start: row.get(1)?, window_capped: row.get(2)?,
            backfill_done: row.get(3)?, highest_uid: row.get(4)?, updated_at: row.get(5)?,
            created: false,
        }),
    ).optional()?)
}

impl Storage {
    /// Every selectable folder on the current source, with a stable order so a
    /// busy account still gives each folder one bounded attempt per round.
    pub(crate) fn sync_folders(
        &self,
        account_id: &str,
        source: &str,
    ) -> Result<Vec<(String, String)>, AppError> {
        let mut statement = self.db.prepare(
            "SELECT f.id, f.remote_name FROM folders f JOIN accounts a ON a.id = f.account_id
             WHERE f.account_id = ?1 AND f.mailbox_source_id = ?2 AND a.mailbox_source_id = ?2
             AND f.selectable = 1 ORDER BY (f.role = 'inbox') DESC, f.remote_name, f.id",
        )?;
        Ok(statement
            .query_map(params![account_id, source], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<_, _>>()?)
    }

    pub(crate) fn update_folder_counts(
        &mut self,
        account_id: &str,
        source: &str,
        folder_id: &str,
        unread: u32,
        total: u32,
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_folder(&tx, account_id, source, folder_id)? {
            return Ok(None);
        }
        // An empty synced_at marks counts as unknown, so filling it in is a change too.
        let changed = tx.execute(
            "UPDATE folders SET unread_count = ?1, total_count = ?2, synced_at = ?3
             WHERE id = ?4 AND (unread_count <> ?1 OR total_count <> ?2 OR synced_at = '')",
            params![unread, total, now(), folder_id],
        )?;
        tx.commit()?;
        Ok(Some(changed))
    }

    pub(crate) fn ensure_generation(
        &mut self,
        target: &SyncTarget,
        window_start: &str,
    ) -> Result<Option<FolderSyncCursor>, AppError> {
        let tx = self.db.transaction()?;
        if !current_folder(
            &tx,
            &target.account_id,
            &target.mailbox_source_id,
            &target.folder_id,
        )? {
            return Ok(None);
        }
        let existing = read_cursor(&tx, &target.folder_id)?;
        if let Some(cursor) = existing.filter(|cursor| cursor.uid_validity == target.uid_validity) {
            return Ok(Some(cursor));
        }
        let timestamp = now();
        tx.execute("UPDATE messages SET remote_state = 'stale', updated_at = ?1 WHERE folder_id = ?2 AND remote_state = 'present'", params![timestamp, target.folder_id])?;
        tx.execute(
            "INSERT INTO folder_sync_cursors (folder_id, uid_validity, window_start, window_capped, backfill_done, highest_uid, updated_at)
             VALUES (?1, ?2, ?3, 0, 0, 0, ?4)
             ON CONFLICT (folder_id) DO UPDATE SET uid_validity = excluded.uid_validity,
             window_start = excluded.window_start, window_capped = 0, backfill_done = 0, highest_uid = 0, updated_at = excluded.updated_at",
            params![target.folder_id, target.uid_validity, window_start, timestamp],
        )?;
        let cursor = read_cursor(&tx, &target.folder_id)?.map(|cursor| FolderSyncCursor {
            created: true,
            ..cursor
        });
        tx.commit()?;
        Ok(cursor)
    }

    pub(crate) fn cached_uids(&self, target: &SyncTarget) -> Result<Vec<(u32, String)>, AppError> {
        if !current_target(&self.db, target)? {
            return Ok(vec![]);
        }
        let mut statement = self.db.prepare("SELECT uid, received_at FROM messages WHERE folder_id = ?1 AND uid_validity = ?2 AND remote_state = 'present' ORDER BY uid")?;
        Ok(statement
            .query_map(params![target.folder_id, target.uid_validity], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<_, _>>()?)
    }

    pub(crate) fn store_message_batch(
        &mut self,
        target: &SyncTarget,
        records: &[MessageRecord],
        highest_uid: u32,
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_target(&tx, target)? {
            return Ok(None);
        }
        let timestamp = now();
        let mut statement = tx.prepare(
            "INSERT INTO messages (id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
             message_id_header, subject, from_name, from_address, to_addresses, cc_addresses, reply_to_addresses,
             received_at, sent_at, size, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, 'present', ?21)
             ON CONFLICT (folder_id, uid_validity, uid) DO UPDATE SET
             message_id_header = excluded.message_id_header, subject = excluded.subject,
             from_name = excluded.from_name, from_address = excluded.from_address,
             to_addresses = excluded.to_addresses, cc_addresses = excluded.cc_addresses, reply_to_addresses = excluded.reply_to_addresses,
             received_at = excluded.received_at, sent_at = excluded.sent_at, size = excluded.size,
             is_read = excluded.is_read, is_flagged = excluded.is_flagged, flags = excluded.flags,
             -- Once the body is downloaded, has_attachment comes from the real MIME parts;
             -- a revived message must not fall back to the header-based estimate.
             has_attachment = CASE WHEN messages.body_state = 'ready' THEN messages.has_attachment ELSE excluded.has_attachment END,
             remote_state = 'present', updated_at = excluded.updated_at",
        )?;
        for record in records {
            statement.execute(params![
                Uuid::new_v4().to_string(),
                target.account_id,
                target.mailbox_source_id,
                target.folder_id,
                target.uid_validity,
                record.uid,
                record.message_id_header,
                record.subject,
                record.from_name,
                record.from_address,
                encode(&record.to_addresses)?,
                encode(&record.cc_addresses)?,
                encode(&record.reply_to_addresses)?,
                record.received_at,
                record.sent_at,
                record.size,
                record.is_read,
                record.is_flagged,
                encode_flags(&record.flags)?,
                record.has_attachment,
                timestamp,
            ])?;
        }
        drop(statement);
        tx.execute("UPDATE folder_sync_cursors SET highest_uid = MAX(highest_uid, ?1), updated_at = ?2 WHERE folder_id = ?3", params![highest_uid, timestamp, target.folder_id])?;
        tx.commit()?;
        // Sync only fetches UIDs that are not cached as present, so each record
        // is a new or revived message.
        Ok(Some(records.len()))
    }

    pub(crate) fn begin_backfill(
        &mut self,
        target: &SyncTarget,
        capped: bool,
    ) -> Result<Option<usize>, AppError> {
        self.set_backfill_state(target, capped, false)
    }

    pub(crate) fn finish_backfill(
        &mut self,
        target: &SyncTarget,
        capped: bool,
    ) -> Result<Option<usize>, AppError> {
        self.set_backfill_state(target, capped, true)
    }

    fn set_backfill_state(
        &mut self,
        target: &SyncTarget,
        capped: bool,
        done: bool,
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_target(&tx, target)? {
            return Ok(None);
        }
        // Coverage depends on these two flags, so only a real transition counts.
        let changed = tx.execute("UPDATE folder_sync_cursors SET backfill_done = ?1, window_capped = ?2, updated_at = ?3 WHERE folder_id = ?4 AND (backfill_done <> ?1 OR window_capped <> ?2)", params![done, capped, now(), target.folder_id])?;
        tx.commit()?;
        Ok(Some(changed))
    }

    pub(crate) fn mark_deleted(
        &mut self,
        target: &SyncTarget,
        uids: &[u32],
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_target(&tx, target)? {
            return Ok(None);
        }
        let timestamp = now();
        let mut changed = 0;
        let mut statement = tx.prepare("UPDATE messages SET remote_state = 'deleted', updated_at = ?1 WHERE folder_id = ?2 AND uid_validity = ?3 AND uid = ?4 AND remote_state = 'present'")?;
        for uid in uids {
            changed += statement.execute(params![
                timestamp,
                target.folder_id,
                target.uid_validity,
                uid
            ])?;
        }
        drop(statement);
        tx.commit()?;
        Ok(Some(changed))
    }

    pub(crate) fn apply_flags(
        &mut self,
        target: &SyncTarget,
        flags: &[(u32, Vec<String>)],
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_target(&tx, target)? {
            return Ok(None);
        }
        let timestamp = now();
        let mut changed = 0;
        // Rows whose flags already match are left alone so the count reflects real changes.
        let mut statement = tx.prepare("UPDATE messages SET is_read = ?1, is_flagged = ?2, flags = ?3, updated_at = ?4 WHERE folder_id = ?5 AND uid_validity = ?6 AND uid = ?7 AND remote_state = 'present' AND (is_read <> ?1 OR is_flagged <> ?2 OR flags <> ?3)")?;
        for (uid, values) in flags {
            changed += statement.execute(params![
                values
                    .iter()
                    .any(|flag| flag.eq_ignore_ascii_case("\\Seen")),
                values
                    .iter()
                    .any(|flag| flag.eq_ignore_ascii_case("\\Flagged")),
                encode_flags(values)?,
                timestamp,
                target.folder_id,
                target.uid_validity,
                uid
            ])?;
        }
        drop(statement);
        tx.commit()?;
        Ok(Some(changed))
    }

    pub(crate) fn list_emails(
        &self,
        validated: &ValidatedEmailQuery,
    ) -> Result<EmailPage, AppError> {
        let query = &validated.query;
        let mut filter = ScopeFilter::new(query)?;
        if let Some(unread) = query.unread {
            filter.add("m.is_read = ?", i64::from(!unread));
        }
        if let Some(attachment) = query.has_attachment {
            filter.add("m.has_attachment = ?", i64::from(attachment));
        }
        if let Some(text) = &query.text {
            let escaped = text
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            filter.clauses.push("(m.subject LIKE ? ESCAPE '\\' OR m.from_name LIKE ? ESCAPE '\\' OR m.from_address LIKE ? ESCAPE '\\')".into());
            filter.values.extend([
                pattern.clone().into(),
                pattern.clone().into(),
                pattern.into(),
            ]);
        }
        if let Some(after) = &query.received_after {
            filter.add("m.received_at >= ?", after.clone());
        }
        if let Some(before) = &query.received_before {
            filter.add("m.received_at < ?", before.clone());
        }
        if let Some(cursor) = &validated.cursor {
            filter.clauses.push("(m.received_at, m.id) < (?, ?)".into());
            filter
                .values
                .extend([cursor.r.clone().into(), cursor.i.clone().into()]);
        }
        filter.values.push((i64::from(validated.limit) + 1).into());
        let sql = format!(
            "SELECT {SUMMARY_COLUMNS} {VISIBLE_MESSAGES} AND {} ORDER BY m.received_at DESC, m.id DESC LIMIT ?",
            filter.sql()
        );
        let mut statement = self.db.prepare(&sql)?;
        let mut items = statement
            .query_map(params_from_iter(filter.values), summary)?
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > validated.limit as usize {
            items.truncate(validated.limit as usize);
            items
                .last()
                .map(|last| {
                    encode(&PageCursor {
                        r: last.received_at.clone(),
                        i: last.id.clone(),
                    })
                    .map(|json| URL_SAFE_NO_PAD.encode(json))
                })
                .transpose()?
        } else {
            None
        };
        Ok(EmailPage {
            items,
            next_cursor,
            coverage: self.email_coverage(query)?,
        })
    }

    fn email_coverage(&self, query: &EmailListQuery) -> Result<Coverage, AppError> {
        let filter = ScopeFilter::new(query)?;
        let sql = format!(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN c.backfill_done = 1 AND c.window_capped = 0 THEN 1 ELSE 0 END), 0), MAX(c.window_start), COALESCE(MAX(c.window_capped), 0)
             FROM folders f JOIN accounts a ON a.id = f.account_id AND a.mailbox_source_id = f.mailbox_source_id
             LEFT JOIN folder_sync_cursors c ON c.folder_id = f.id WHERE {}", filter.sql());
        let (total, complete, synced_after, window_capped): (i64, i64, Option<String>, bool) = self
            .db
            .query_row(&sql, params_from_iter(filter.values), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
        Ok(Coverage {
            headers_complete: total > 0 && total == complete,
            window_capped,
            bodies_complete: false,
            synced_after,
            synced_before: None,
        })
    }

    pub(crate) fn get_email(&self, id: &str) -> Result<EmailDetail, AppError> {
        let sql = format!(
            "SELECT {SUMMARY_COLUMNS}, m.to_addresses, m.cc_addresses, m.reply_to_addresses, m.body_failure {VISIBLE_MESSAGES} AND m.id = ?1"
        );
        let result = self
            .db
            .query_row(&sql, [id], |row| {
                Ok((
                    summary(row)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, Option<String>>(15)?,
                ))
            })
            .optional()?;
        let (summary, to, cc, reply_to, body_failure) =
            result.ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        let body_failure = body_failure
            .map(|failure| serde_json::from_value(serde_json::Value::String(failure)))
            .transpose()
            .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
        let (body, attachments) = if summary.body_state == BodyState::Ready {
            // Do not select HTML even internally on the ordinary IPC read path.
            let body = self.db.query_row(
                "SELECT plain_text, sanitized_html IS NOT NULL, truncated, remote_images_blocked
                 FROM message_bodies WHERE message_id = ?1", [id], |row| {
                    Ok(EmailBody {
                        text: row.get(0)?, has_html: row.get(1)?, truncated: row.get(2)?,
                        remote_images_blocked: row.get(3)?,
                    })
                },
            ).optional()?.ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
            let mut statement = self.db.prepare(
                "SELECT part_index, filename, mime_type, size FROM message_parts
                 WHERE message_id = ?1 AND is_inline = 0 ORDER BY part_index",
            )?;
            let attachments = statement
                .query_map([id], |row| {
                    let index: i64 = row.get(0)?;
                    let size: i64 = row.get(3)?;
                    let size = u64::try_from(size)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(3, size))?;
                    Ok(AttachmentSummary {
                        id: format!("{id}:{index}"),
                        filename: row
                            .get::<_, Option<String>>(1)?
                            .unwrap_or_else(|| "未命名附件".into()),
                        mime_type: row.get(2)?,
                        size,
                        is_inline: false,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            (Some(body), attachments)
        } else {
            (None, vec![])
        };
        Ok(EmailDetail {
            summary,
            to: decode(&to)?,
            cc: decode(&cc)?,
            reply_to: decode(&reply_to)?,
            body,
            body_failure,
            attachments,
        })
    }
}

const SUMMARY_COLUMNS: &str = "m.id, m.account_id, m.folder_id, m.subject, m.from_name, m.from_address, m.received_at, m.is_read, m.is_flagged, m.has_attachment, m.preview, m.body_state";
const VISIBLE_MESSAGES: &str = "FROM messages m JOIN accounts a ON a.id = m.account_id AND a.mailbox_source_id = m.mailbox_source_id
 JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id AND f.mailbox_source_id = m.mailbox_source_id
 JOIN folder_sync_cursors c ON c.folder_id = m.folder_id AND c.uid_validity = m.uid_validity WHERE m.remote_state = 'present' AND f.selectable = 1";

fn summary(row: &Row<'_>) -> rusqlite::Result<EmailSummary> {
    Ok(EmailSummary {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        subject: row.get(3)?,
        from: MailAddress {
            name: row.get(4)?,
            address: row.get(5)?,
        },
        received_at: row.get(6)?,
        is_read: row.get(7)?,
        is_starred: row.get(8)?,
        has_attachment: row.get(9)?,
        preview: row.get(10)?,
        body_state: match row.get::<_, String>(11)?.as_str() {
            "missing" => BodyState::Missing,
            "fetching" => BodyState::Fetching,
            "ready" => BodyState::Ready,
            "failed" => BodyState::Failed,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        pending_changes: false,
    })
}

/// Shared account/folder scope for message rows and coverage; filters on message
/// contents must never hide an unsynchronized folder from the coverage calculation.
struct ScopeFilter {
    clauses: Vec<String>,
    values: Vec<Value>,
}

impl ScopeFilter {
    fn new(query: &EmailListQuery) -> Result<Self, AppError> {
        let mut result = Self {
            clauses: vec!["f.selectable = 1".into()],
            values: vec![],
        };
        if let Some(accounts) = &query.account_ids {
            if accounts.is_empty() {
                result.clauses.push("0 = 1".into());
            } else {
                result.clauses.push(format!(
                    "f.account_id IN ({})",
                    vec!["?"; accounts.len()].join(",")
                ));
                result
                    .values
                    .extend(accounts.iter().cloned().map(Value::from));
            }
        }
        if let Some(folder) = &query.folder_id {
            result.add("f.id = ?", folder.clone());
        }
        if let Some(role) = query.folder_role {
            let role = encode(&role)?;
            result.add("f.role = ?", role.trim_matches('"').to_owned());
        }
        Ok(result)
    }

    fn add(&mut self, clause: &str, value: impl Into<Value>) {
        self.clauses.push(clause.into());
        self.values.push(value.into());
    }

    fn sql(&self) -> String {
        self.clauses.join(" AND ")
    }
}

#[cfg(test)]
mod tests;
