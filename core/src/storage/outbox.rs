//! Send queue rows (docs/architecture/sending.md sections 5, 6 and 10).
//!
//! Every state change is a single guarded `UPDATE ... WHERE state = ...`, so
//! the worker starting an attempt and the user canceling cannot both win:
//! whichever statement runs second matches no row.
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};

use super::{
    Storage,
    bodies::{BlobStore, StoredBlob},
    decode, encode,
};
use crate::{
    AppError, ErrorCode,
    send::clock::stamp,
    send::types::{
        DraftChange, DraftChanged, OutboxError, OutboxItem, OutboxResolution, OutboxState,
        ServerReplyInfo,
    },
    smtp::message::SmtpEnvelope,
};

/// Everything but the MIME bytes.
const COLUMNS: &str = "id, draft_id, account_id, message_id, subject, recipients_preview,
    recipient_count, state, not_before, data_started_at, attempts, next_attempt_at,
    first_queued_at, last_error, accepted_at, server_reply, server_copy_message_id,
    resolution, superseded_by, created_at, updated_at";

/// A draft is held by the queue while one of its items is in flight, waiting,
/// or an `unknown` the user has not settled.
const LIVE: &str = "(state IN ('queued', 'sending') OR (state = 'unknown' AND resolution IS NULL))";

pub(crate) struct NewOutbox<'a> {
    pub id: &'a str,
    pub draft_id: Option<&'a str>,
    pub account_id: &'a str,
    pub client_mutation_id: &'a str,
    pub canonical: &'a str,
    pub message_id: &'a str,
    pub snapshot: &'a StoredBlob,
    /// What the snapshot was built from; the insert fails if the draft moved on.
    pub frozen: Option<&'a FrozenDraft>,
    pub envelope: &'a SmtpEnvelope,
    pub subject: &'a str,
    pub recipients_preview: &'a [String],
    /// Plain-text excerpt for the local Sent copy row (sending.md section 9.2).
    pub preview: &'a str,
    /// Regular (non-inline) attachments frozen into the snapshot.
    pub attachment_count: u32,
    pub not_before: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

/// The draft state a snapshot was built from (sending.md section 16.6).
pub(crate) struct FrozenDraft {
    pub revision: u32,
    /// Every regular attachment and the inline images the body references.
    pub attachment_ids: Vec<String>,
}

/// What the worker needs to attempt one item.
pub(crate) struct ClaimedJob {
    pub item: OutboxItem,
    /// The frozen MIME bytes; `None` when the row has no snapshot (released or lost).
    pub snapshot: Option<StoredBlob>,
    pub blobs: BlobStore,
    pub envelope: SmtpEnvelope,
}

pub(crate) enum Claim {
    Job(Box<ClaimedJob>),
    /// Nothing is due; the earliest time something becomes due, if anything is queued.
    Wait(Option<DateTime<Utc>>),
}

/// The effect of a state change the host should hear about.
#[derive(Debug)]
pub(crate) struct Changed {
    pub item: OutboxItem,
    pub draft: Option<DraftChanged>,
}

fn parse(row: &Row<'_>) -> rusqlite::Result<Result<OutboxItem, AppError>> {
    let preview: String = row.get(5)?;
    let state: String = row.get(7)?;
    let last_error: Option<String> = row.get(13)?;
    let reply: Option<String> = row.get(15)?;
    let resolution: Option<String> = row.get(17)?;
    let base = (
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(6)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(14)?,
        row.get(16)?,
        row.get(18)?,
        row.get(19)?,
        row.get(20)?,
    );
    #[allow(clippy::type_complexity)]
    let base: (
        String,
        Option<String>,
        String,
        String,
        String,
        u32,
        String,
        Option<String>,
        u32,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
    ) = base;
    Ok((|| {
        Ok(OutboxItem {
            id: base.0,
            draft_id: base.1,
            account_id: base.2,
            message_id: base.3,
            subject: base.4,
            recipients: decode(&preview)?,
            recipient_count: base.5,
            state: OutboxState::parse(&state)
                .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?,
            not_before: base.6,
            data_started_at: base.7,
            attempts: base.8,
            next_attempt_at: base.9,
            first_queued_at: base.10,
            error: last_error.map(|text| decode(&text)).transpose()?,
            accepted_at: base.11,
            server_reply: reply.map(|text| decode(&text)).transpose()?,
            server_copy_message_id: base.12,
            resolution: resolution
                .map(|text| {
                    OutboxResolution::parse(&text)
                        .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))
                })
                .transpose()?,
            superseded_by: base.13,
            created_at: base.14,
            updated_at: base.15,
        })
    })())
}

fn item_in(db: &rusqlite::Connection, id: &str) -> Result<OutboxItem, AppError> {
    db.query_row(
        &format!("SELECT {COLUMNS} FROM outbox WHERE id = ?1"),
        [id],
        parse,
    )
    .optional()?
    .ok_or_else(|| AppError::new(ErrorCode::NotFound))?
}

pub(super) fn live_for_draft(db: &rusqlite::Connection, draft_id: &str) -> Result<bool, AppError> {
    Ok(db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM outbox WHERE draft_id = ?1 AND {LIVE})"),
        [draft_id],
        |row| row.get(0),
    )?)
}

/// `Some` when the draft still exists and the queue just let go of it.
fn unlocked(item: &OutboxItem) -> Option<DraftChanged> {
    item.draft_id.as_ref().map(|draft_id| DraftChanged {
        draft_id: draft_id.clone(),
        account_id: item.account_id.clone(),
        change: DraftChange::Unlocked,
    })
}

/// Lets go of the snapshots of a draft's failed and canceled items: nothing
/// resends them, and the next cleanup may free the blobs.
fn release_snapshots(tx: &rusqlite::Connection, draft_id: &str, now: &str) -> Result<(), AppError> {
    tx.execute(
        "UPDATE outbox SET snapshot_hash = NULL, snapshot_size = NULL, snapshot_released_at = ?2
         WHERE draft_id = ?1 AND state IN ('failed', 'canceled') AND snapshot_hash IS NOT NULL",
        params![draft_id, now],
    )?;
    Ok(())
}

/// Header sync of a `sent` folder: every accepted item of the account whose
/// Message-ID is among `message_ids` (as the parser reports them, without angle
/// brackets) now has its server copy. Records that copy and lets go of the
/// snapshot (sending.md section 16.7). Matches by Message-ID only, never by
/// subject or time, and only items already settled as `accepted`: `queued`,
/// `sending` and `unknown` keep their snapshot. Idempotent: an item with a
/// recorded copy is not touched again, so nothing is released twice or revived.
/// Returns how many items were matched.
pub(super) fn release_matched_snapshots(
    tx: &rusqlite::Connection,
    account_id: &str,
    message_ids: &[&str],
    now: &str,
) -> Result<usize, AppError> {
    let mut statement = tx.prepare_cached(
        "UPDATE outbox SET server_copy_message_id = message_id, snapshot_hash = NULL,
             snapshot_size = NULL, snapshot_released_at = ?3
         WHERE account_id = ?1 AND state = 'accepted'
           AND trim(message_id, '<>') = ?2 AND server_copy_message_id IS NULL",
    )?;
    let mut matched = 0;
    for message_id in message_ids {
        // `message_ids` come without angle brackets; the outbox keeps the
        // header as it was sent, with them.
        matched += statement.execute(params![account_id, message_id, now])?;
    }
    Ok(matched)
}

impl Storage {
    /// Moves snapshots that predate migration 14 out of `outbox.mime` into the
    /// blob store. Idempotent: the blob is written (and synced) before the row
    /// is touched, so a crash leaves either the old row or the new one.
    pub(crate) fn migrate_outbox_snapshots(&mut self) -> Result<(), AppError> {
        loop {
            let pending: Option<(String, Vec<u8>)> = self
                .db
                .query_row(
                    "SELECT id, mime FROM outbox
                     WHERE mime IS NOT NULL ORDER BY created_at, id LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((id, mime)) = pending else {
                return Ok(());
            };
            let blob = self.blobs.put(&mime)?;
            self.db.execute(
                "UPDATE outbox SET snapshot_hash = ?2, snapshot_size = ?3, mime = NULL
                 WHERE id = ?1",
                params![
                    id,
                    blob.sha256,
                    i64::try_from(blob.size).unwrap_or(i64::MAX)
                ],
            )?;
        }
    }

    /// Fills the Sent-list fields of accepted items queued before migration 18
    /// from their snapshot (sending.md section 9.2). Rows whose snapshot is
    /// released are left alone: they are not shown. A snapshot that cannot be
    /// read or parsed gets an empty excerpt so it is not retried at every
    /// start. Idempotent: only rows still missing a field are visited.
    pub(crate) fn backfill_outbox_list_fields(&mut self) -> Result<(), AppError> {
        let pending: Vec<(String, String, i64, String)> = self
            .db
            .prepare(
                "SELECT id, snapshot_hash, snapshot_size, message_id FROM outbox
                 WHERE state = 'accepted' AND snapshot_hash IS NOT NULL
                   AND snapshot_released_at IS NULL
                   AND (preview IS NULL OR attachment_count IS NULL)
                 ORDER BY created_at, id",
            )?
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<_, _>>()?;
        for (id, sha256, size, message_id) in pending {
            let blob = StoredBlob {
                sha256,
                size: u64::try_from(size).unwrap_or(u64::MAX),
            };
            let (preview, count) = self
                .blobs
                .read(&blob)
                .ok()
                .and_then(|raw| crate::mail::render::render(&raw, &message_id).ok())
                .map(|rendered| {
                    (
                        crate::mail::render::preview(rendered.plain_text.as_deref()),
                        rendered.parts.iter().filter(|part| !part.is_inline).count(),
                    )
                })
                .unwrap_or_default();
            self.db.execute(
                "UPDATE outbox SET preview = ?2, attachment_count = ?3
                 WHERE id = ?1 AND (preview IS NULL OR attachment_count IS NULL)",
                params![id, preview, i64::try_from(count).unwrap_or(i64::MAX)],
            )?;
        }
        Ok(())
    }

    pub(crate) fn draft_locked(&self, draft_id: &str) -> Result<bool, AppError> {
        live_for_draft(&self.db, draft_id)
    }

    /// The account's address and sender name as they are now.
    pub(crate) fn account_sender(
        &self,
        account_id: &str,
    ) -> Result<(String, Option<String>), AppError> {
        self.db
            .query_row(
                "SELECT email_address, sender_name FROM accounts WHERE id = ?1",
                [account_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))
    }

    /// The item a mutation id queued, with the canonical form of its request.
    pub(crate) fn outbox_by_mutation(
        &self,
        client_mutation_id: &str,
    ) -> Result<Option<(OutboxItem, String)>, AppError> {
        let found = self
            .db
            .query_row(
                &format!(
                    "SELECT {COLUMNS}, queue_canonical FROM outbox
                     WHERE client_mutation_id = ?1 ORDER BY created_at, id LIMIT 1"
                ),
                [client_mutation_id],
                |row| Ok((parse(row)?, row.get::<_, String>(21)?)),
            )
            .optional()?;
        found
            .map(|(item, canonical)| Ok((item?, canonical)))
            .transpose()
    }

    /// Inserts a queued item and, in the same transaction, fails if the draft
    /// already has a live one. The draft is thereby locked.
    pub(crate) fn insert_outbox(&mut self, new: &NewOutbox<'_>) -> Result<OutboxItem, AppError> {
        let tx = self.db.transaction()?;
        if let Some(draft_id) = new.draft_id
            && live_for_draft(&tx, draft_id)?
        {
            return Err(AppError::new(ErrorCode::DraftLocked));
        }
        if let (Some(draft_id), Some(frozen)) = (new.draft_id, new.frozen) {
            let revision: Option<u32> = tx
                .query_row(
                    "SELECT revision FROM drafts WHERE id = ?1",
                    [draft_id],
                    |row| row.get(0),
                )
                .optional()?;
            let ids = {
                let mut statement = tx.prepare(
                    "SELECT id, inline FROM draft_attachments WHERE draft_id = ?1 ORDER BY rowid",
                )?;
                statement
                    .query_map([draft_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    // Inline images the body no longer shows were not frozen.
                    .filter(|(id, inline)| !inline || frozen.attachment_ids.contains(id))
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>()
            };
            // The snapshot is stale: the orphan blob is left for the next cleanup.
            if revision != Some(frozen.revision) || ids != frozen.attachment_ids {
                return Err(AppError::new(ErrorCode::Conflict));
            }
        }
        let now = stamp(new.now);
        tx.execute(
            "INSERT INTO outbox (id, draft_id, account_id, client_mutation_id, queue_canonical,
                message_id, snapshot_hash, snapshot_size, envelope, subject, recipients_preview,
                recipient_count, state, not_before, attempts, first_queued_at, created_at,
                updated_at, preview, attachment_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?14, ?8, ?9, ?10, ?11, 'queued', ?12, 0, ?13, ?13, ?13,
                ?15, ?16)",
            params![
                new.id,
                new.draft_id,
                new.account_id,
                new.client_mutation_id,
                new.canonical,
                new.message_id,
                new.snapshot.sha256,
                encode(new.envelope)?,
                new.subject,
                encode(&new.recipients_preview)?,
                new.envelope.recipients.len() as i64,
                stamp(new.not_before),
                now,
                i64::try_from(new.snapshot.size).unwrap_or(i64::MAX),
                new.preview,
                i64::from(new.attachment_count),
            ],
        )?;
        if let Some(draft_id) = new.draft_id {
            // A re-queue supersedes this draft's earlier failed or canceled tries.
            release_snapshots(&tx, draft_id, &now)?;
        }
        let item = item_in(&tx, new.id)?;
        tx.commit()?;
        Ok(item)
    }

    pub(crate) fn get_outbox(&self, id: &str) -> Result<OutboxItem, AppError> {
        item_in(&self.db, id)
    }

    /// Newest first. Without `include_all`: items still needing attention or
    /// time (queued, sending, failed, unsettled unknown).
    pub(crate) fn list_outbox(
        &self,
        account_id: Option<&str>,
        include_all: bool,
    ) -> Result<Vec<OutboxItem>, AppError> {
        let mut statement = self.db.prepare(&format!(
            "SELECT {COLUMNS} FROM outbox
             WHERE (?1 IS NULL OR account_id = ?1)
               AND (?2 OR state IN ('queued', 'sending', 'failed')
                    OR (state = 'unknown' AND resolution IS NULL))
             ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = statement
            .query_map(params![account_id, include_all], parse)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().collect()
    }

    /// Fails queued items whose envelope sender is no longer the account's
    /// address (sending.md section 5, `accountChanged`).
    pub(crate) fn fail_changed_sender(
        &mut self,
        account_id: &str,
        current_address: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<Changed>, AppError> {
        let tx = self.db.transaction()?;
        let queued = {
            let mut statement = tx.prepare(
                "SELECT id, envelope FROM outbox WHERE account_id = ?1 AND state = 'queued'",
            )?;
            statement
                .query_map([account_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut changed = Vec::new();
        for (id, envelope) in queued {
            let envelope: SmtpEnvelope = decode(&envelope)?;
            if envelope.from.eq_ignore_ascii_case(current_address) {
                continue;
            }
            let error = OutboxError::reason("accountChanged");
            let rows = tx.execute(
                "UPDATE outbox SET state = 'failed', next_attempt_at = NULL, last_error = ?2,
                    updated_at = ?3 WHERE id = ?1 AND state = 'queued'",
                params![id, encode(&error)?, stamp(now)],
            )?;
            if rows == 1 {
                let item = item_in(&tx, &id)?;
                changed.push(Changed {
                    draft: unlocked(&item),
                    item,
                });
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Takes the earliest due queued item into `sending`, or says when to look
    /// again. Only a still-`queued` row can be taken.
    pub(crate) fn claim_next(
        &mut self,
        account_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(Claim, Option<Changed>), AppError> {
        let tx = self.db.transaction()?;
        let stamp_now = stamp(now);
        loop {
            let due: Option<String> = tx
                .query_row(
                    "SELECT id FROM outbox
                     WHERE account_id = ?1 AND state = 'queued' AND not_before <= ?2
                       AND (next_attempt_at IS NULL OR next_attempt_at <= ?2)
                     ORDER BY COALESCE(next_attempt_at, not_before), created_at, id LIMIT 1",
                    params![account_id, stamp_now],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(id) = due else {
                let next: Option<String> = tx.query_row(
                    "SELECT MIN(MAX(not_before, COALESCE(next_attempt_at, not_before)))
                     FROM outbox WHERE account_id = ?1 AND state = 'queued'",
                    [account_id],
                    |row| row.get(0),
                )?;
                let next = next.and_then(|text| crate::send::clock::parse_stamp(&text));
                return Ok((Claim::Wait(next), None));
            };
            let rows = tx.execute(
                "UPDATE outbox SET state = 'sending', attempts = attempts + 1,
                    next_attempt_at = NULL, updated_at = ?2
                 WHERE id = ?1 AND state = 'queued'",
                params![id, stamp_now],
            )?;
            if rows == 0 {
                continue;
            }
            let (hash, size, envelope): (Option<String>, Option<i64>, String) = tx.query_row(
                "SELECT snapshot_hash, snapshot_size, envelope FROM outbox WHERE id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            let item = item_in(&tx, &id)?;
            tx.commit()?;
            let changed = Changed {
                item: item.clone(),
                draft: None,
            };
            return Ok((
                Claim::Job(Box::new(ClaimedJob {
                    item,
                    snapshot: hash
                        .zip(size.and_then(|size| u64::try_from(size).ok()))
                        .map(|(sha256, size)| StoredBlob { sha256, size }),
                    blobs: self.blobs.clone(),
                    envelope: decode(&envelope)?,
                })),
                Some(changed),
            ));
        }
    }

    /// Persists that the message body is about to be written. Must be durable
    /// before the first body byte is sent.
    pub(crate) fn mark_data_started(&self, id: &str, now: DateTime<Utc>) -> Result<(), AppError> {
        let rows = self.db.execute(
            "UPDATE outbox SET data_started_at = ?2, updated_at = ?2
             WHERE id = ?1 AND state = 'sending'",
            params![id, stamp(now)],
        )?;
        if rows == 1 {
            Ok(())
        } else {
            Err(AppError::new(ErrorCode::Conflict))
        }
    }

    /// `sending` back to `queued`, to be tried again at `at`.
    pub(crate) fn requeue(
        &self,
        id: &str,
        at: DateTime<Utc>,
        error: &OutboxError,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        self.finish_sending(id, now, |tx| {
            tx.execute(
                "UPDATE outbox SET state = 'queued', next_attempt_at = ?2, last_error = ?3,
                    updated_at = ?4 WHERE id = ?1 AND state = 'sending'",
                params![id, stamp(at), encode(error)?, stamp(now)],
            )
            .map_err(Into::into)
        })
    }

    pub(crate) fn fail(
        &self,
        id: &str,
        error: &OutboxError,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        self.finish_sending(id, now, |tx| {
            tx.execute(
                "UPDATE outbox SET state = 'failed', next_attempt_at = NULL, last_error = ?2,
                    updated_at = ?3 WHERE id = ?1 AND state = 'sending'",
                params![id, encode(error)?, stamp(now)],
            )
            .map_err(Into::into)
        })
        .map(|changed| {
            changed.map(|mut changed| {
                changed.draft = unlocked(&changed.item);
                changed
            })
        })
    }

    pub(crate) fn mark_unknown(
        &self,
        id: &str,
        error: &OutboxError,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        self.finish_sending(id, now, |tx| {
            tx.execute(
                "UPDATE outbox SET state = 'unknown', next_attempt_at = NULL, last_error = ?2,
                    updated_at = ?3 WHERE id = ?1 AND state = 'sending'",
                params![id, encode(error)?, stamp(now)],
            )
            .map_err(Into::into)
        })
    }

    /// `accepted`: the row becomes the local Sent copy and the draft is deleted
    /// in the same transaction.
    pub(crate) fn accept(
        &self,
        id: &str,
        reply: &ServerReplyInfo,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        let tx = self.db.unchecked_transaction()?;
        let changed = accept_in(&tx, id, Some(reply), now, "sending")?;
        tx.commit()?;
        Ok(changed)
    }

    fn finish_sending(
        &self,
        id: &str,
        _now: DateTime<Utc>,
        update: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<usize, AppError>,
    ) -> Result<Option<Changed>, AppError> {
        let tx = self.db.unchecked_transaction()?;
        if update(&tx)? == 0 {
            // The row vanished (account deleted) or is no longer ours.
            return Ok(None);
        }
        let item = item_in(&tx, id)?;
        tx.commit()?;
        Ok(Some(Changed { item, draft: None }))
    }

    /// `queued` to `canceled`. The only way out of `queued` besides the worker's
    /// own guarded claim; a loser sees why.
    pub(crate) fn cancel_queued(&self, id: &str, now: DateTime<Utc>) -> Result<Changed, AppError> {
        let tx = self.db.unchecked_transaction()?;
        let rows = tx.execute(
            "UPDATE outbox SET state = 'canceled', next_attempt_at = NULL, updated_at = ?2
             WHERE id = ?1 AND state = 'queued'",
            params![id, stamp(now)],
        )?;
        let item = item_in(&tx, id)?;
        if rows == 0 {
            return Err(AppError::new(if item.state == OutboxState::Sending {
                ErrorCode::AlreadySending
            } else {
                ErrorCode::Conflict
            }));
        }
        tx.commit()?;
        Ok(Changed {
            draft: unlocked(&item),
            item,
        })
    }

    /// "Delete" on a local Sent copy (sending.md section 9.4): hides the item
    /// and releases its snapshot like a server match does, but keeps
    /// `server_copy_message_id` empty. `None` when it was already removed;
    /// `notFound` when the item is not an accepted, unmatched local copy.
    pub(crate) fn remove_local_copy(
        &self,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Changed>, AppError> {
        let tx = self.db.unchecked_transaction()?;
        let stamped = stamp(now);
        let rows = tx.execute(
            "UPDATE outbox SET deleted_locally_at = ?2, snapshot_hash = NULL, snapshot_size = NULL,
                    snapshot_released_at = ?2, updated_at = ?2
             WHERE id = ?1 AND state = 'accepted' AND server_copy_message_id IS NULL
               AND deleted_locally_at IS NULL AND snapshot_released_at IS NULL",
            params![id, stamped],
        )?;
        if rows == 0 {
            let removed: Option<bool> = tx
                .query_row(
                    "SELECT state = 'accepted' AND server_copy_message_id IS NULL
                            AND deleted_locally_at IS NOT NULL
                     FROM outbox WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?;
            return if removed == Some(true) {
                Ok(None)
            } else {
                Err(AppError::new(ErrorCode::NotFound))
            };
        }
        let item = item_in(&tx, id)?;
        tx.commit()?;
        Ok(Some(Changed { item, draft: None }))
    }

    /// Clears the backoff wait of a queued item.
    pub(crate) fn retry_now(&self, id: &str, now: DateTime<Utc>) -> Result<OutboxItem, AppError> {
        let tx = self.db.unchecked_transaction()?;
        let rows = tx.execute(
            "UPDATE outbox SET next_attempt_at = NULL, updated_at = ?2
             WHERE id = ?1 AND state = 'queued'",
            params![id, stamp(now)],
        )?;
        let item = item_in(&tx, id)?;
        if rows == 0 {
            return Err(AppError::new(if item.state == OutboxState::Sending {
                ErrorCode::AlreadySending
            } else {
                ErrorCode::Conflict
            }));
        }
        tx.commit()?;
        Ok(item)
    }

    /// Startup recovery. `sending` without a body-start mark proves the server
    /// got nothing: back to `queued`, and that try does not count. With the
    /// mark the server may have the message: `unknown`.
    pub(crate) fn recover_outbox(&mut self, now: DateTime<Utc>) -> Result<Vec<Changed>, AppError> {
        let tx = self.db.transaction()?;
        let stale = {
            let mut statement = tx.prepare("SELECT id FROM outbox WHERE state = 'sending'")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut changed = Vec::new();
        for id in stale {
            let interrupted = OutboxError::reason("interrupted");
            tx.execute(
                "UPDATE outbox SET
                    state = CASE WHEN data_started_at IS NULL THEN 'queued' ELSE 'unknown' END,
                    attempts = CASE WHEN data_started_at IS NULL
                        THEN MAX(attempts - 1, 0) ELSE attempts END,
                    last_error = CASE WHEN data_started_at IS NULL THEN last_error ELSE ?2 END,
                    next_attempt_at = NULL, updated_at = ?3
                 WHERE id = ?1 AND state = 'sending'",
                params![id, encode(&interrupted)?, stamp(now)],
            )?;
            changed.push(Changed {
                item: item_in(&tx, &id)?,
                draft: None,
            });
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Declares an `unknown` item delivered.
    pub(crate) fn mark_sent(
        &mut self,
        id: &str,
        mutation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Changed, AppError> {
        let tx = self.db.transaction()?;
        let item = item_in(&tx, id)?;
        if item.state != OutboxState::Unknown || item.resolution.is_some() {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let changed = accept_in(&tx, id, None, now, "unknown")?
            .ok_or_else(|| AppError::new(ErrorCode::Conflict))?;
        tx.execute(
            "UPDATE outbox SET resolution = 'markedSent', resolution_mutation_id = ?2 WHERE id = ?1",
            params![id, mutation_id],
        )?;
        let item = item_in(&tx, id)?;
        tx.commit()?;
        Ok(Changed {
            item,
            draft: changed.draft,
        })
    }

    /// Queues a new attempt for an `unknown` item: same Message-ID, same bytes.
    /// The old row is marked `resent` and points at the new one.
    pub(crate) fn resend_unknown(
        &mut self,
        id: &str,
        mutation_id: &str,
        new_id: &str,
        canonical: &str,
        now: DateTime<Utc>,
    ) -> Result<Changed, AppError> {
        let tx = self.db.transaction()?;
        let old = item_in(&tx, id)?;
        if old.state != OutboxState::Unknown || old.resolution.is_some() {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let stamp_now = stamp(now);
        tx.execute(
            "INSERT INTO outbox (id, draft_id, account_id, client_mutation_id, queue_canonical,
                message_id, snapshot_hash, snapshot_size, envelope, subject, recipients_preview,
                recipient_count, state, not_before, attempts, first_queued_at, created_at,
                updated_at)
             SELECT ?2, draft_id, account_id, ?3, ?4, message_id, snapshot_hash, snapshot_size,
                    envelope, subject,
                    recipients_preview, recipient_count, 'queued', ?5, 0, ?5, ?5, ?5
             FROM outbox WHERE id = ?1",
            params![id, new_id, mutation_id, canonical, stamp_now],
        )?;
        tx.execute(
            "UPDATE outbox SET resolution = 'resent', resolution_mutation_id = ?2,
                superseded_by = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, mutation_id, new_id, stamp_now],
        )?;
        let item = item_in(&tx, new_id)?;
        tx.commit()?;
        Ok(Changed { item, draft: None })
    }

    /// Replays a resolution: the item a settled `unknown` row led to.
    pub(crate) fn resolution_replay(
        &self,
        id: &str,
        mutation_id: &str,
    ) -> Result<Option<OutboxItem>, AppError> {
        let item = item_in(&self.db, id)?;
        let stored: Option<String> = self.db.query_row(
            "SELECT resolution_mutation_id FROM outbox WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if stored.as_deref() != Some(mutation_id) {
            return Ok(None);
        }
        match &item.superseded_by {
            Some(next) => Ok(Some(item_in(&self.db, next)?)),
            None => Ok(Some(item)),
        }
    }
}

/// Shared by `accept` and `mark_sent`: state change, server reply, draft delete.
fn accept_in(
    tx: &rusqlite::Connection,
    id: &str,
    reply: Option<&ServerReplyInfo>,
    now: DateTime<Utc>,
    from: &str,
) -> Result<Option<Changed>, AppError> {
    let before = item_in(tx, id)?;
    let rows = tx.execute(
        "UPDATE outbox SET state = 'accepted', accepted_at = ?3, server_reply = ?4,
            next_attempt_at = NULL, last_error = NULL, updated_at = ?3
         WHERE id = ?1 AND state = ?2",
        params![id, from, stamp(now), reply.map(encode).transpose()?],
    )?;
    if rows == 0 {
        return Ok(None);
    }
    let mut draft = None;
    if let Some(draft_id) = &before.draft_id
        && tx.execute("DELETE FROM drafts WHERE id = ?1", [draft_id])? == 1
    {
        draft = Some(DraftChanged {
            draft_id: draft_id.clone(),
            account_id: before.account_id.clone(),
            change: DraftChange::Deleted,
        });
    }
    Ok(Some(Changed {
        item: item_in(tx, id)?,
        draft,
    }))
}
