//! Draft attachment rows (docs/architecture/sending.md section 16.3).
//!
//! Adding or removing one is a compare-and-swap on the draft `revision`, like a
//! body edit, and is refused while the send queue holds the draft. The bytes
//! are in the blob store; a row only names the blob.
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Storage, bodies::StoredBlob, outbox::live_for_draft};
use crate::{
    AppError, AttachmentOrigin, AttachmentPolicy, Draft, DraftAttachmentSummary, ErrorCode,
};

const COLUMNS: &str = "id, file_name, media_type, size, blob_hash, origin";

pub(crate) struct NewAttachment<'a> {
    pub id: &'a str,
    pub file_name: &'a str,
    pub media_type: &'a str,
    pub blob: &'a StoredBlob,
    pub origin: AttachmentOrigin,
}

/// An attachment together with the blob holding its bytes.
pub(crate) struct AttachmentFile {
    pub summary: DraftAttachmentSummary,
    pub blob: StoredBlob,
}

fn parse(row: &Row<'_>) -> rusqlite::Result<AttachmentFile> {
    let origin: String = row.get(5)?;
    let size = u64::try_from(row.get::<_, i64>(3)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(AttachmentFile {
        summary: DraftAttachmentSummary {
            id: row.get(0)?,
            file_name: row.get(1)?,
            media_type: row.get(2)?,
            size,
            origin: AttachmentOrigin::parse(&origin).ok_or(rusqlite::Error::InvalidQuery)?,
        },
        blob: StoredBlob {
            sha256: row.get(4)?,
            size,
        },
    })
}

fn files_of(db: &Connection, draft_id: &str) -> Result<Vec<AttachmentFile>, AppError> {
    let mut statement = db.prepare(&format!(
        "SELECT {COLUMNS} FROM draft_attachments WHERE draft_id = ?1 ORDER BY rowid"
    ))?;
    let files = statement
        .query_map([draft_id], parse)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
    Ok(files)
}

/// Bumps the revision for an attachment change; `NotFound`, `Conflict` and
/// `DraftLocked` are decided here, inside the caller's transaction.
fn begin_change(
    tx: &Connection,
    draft_id: &str,
    expected_revision: u32,
    now: &str,
) -> Result<(), AppError> {
    let current: Option<u32> = tx
        .query_row(
            "SELECT revision FROM drafts WHERE id = ?1",
            [draft_id],
            |row| row.get(0),
        )
        .optional()?;
    let current = current.ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
    if live_for_draft(tx, draft_id)? {
        return Err(AppError::new(ErrorCode::DraftLocked));
    }
    if current != expected_revision {
        return Err(AppError::new(ErrorCode::Conflict));
    }
    let next = current
        .checked_add(1)
        .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
    tx.execute(
        "UPDATE drafts SET revision = ?2, updated_at = ?3 WHERE id = ?1",
        params![draft_id, next, now],
    )?;
    Ok(())
}

impl Storage {
    pub(crate) fn attachment_files(&self, draft_id: &str) -> Result<Vec<AttachmentFile>, AppError> {
        files_of(&self.db, draft_id)
    }

    pub(crate) fn with_attachments(&self, mut draft: Draft) -> Result<Draft, AppError> {
        draft.attachments = files_of(&self.db, &draft.id)?
            .into_iter()
            .map(|file| file.summary)
            .collect();
        Ok(draft)
    }

    pub(crate) fn add_attachment(
        &mut self,
        draft_id: &str,
        expected_revision: u32,
        attachment: &NewAttachment<'_>,
        policy: AttachmentPolicy,
        now: &str,
    ) -> Result<Draft, AppError> {
        let tx = self.db.transaction()?;
        begin_change(&tx, draft_id, expected_revision, now)?;
        let (count, total): (i64, i64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM draft_attachments WHERE draft_id = ?1",
            [draft_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if count >= i64::from(policy.max_count) {
            return Err(AppError::new(ErrorCode::TooManyAttachments));
        }
        let size = i64::try_from(attachment.blob.size)
            .map_err(|_| AppError::new(ErrorCode::AttachmentTooLarge))?;
        if u64::try_from(total.saturating_add(size)).unwrap_or(u64::MAX) > policy.max_total_bytes {
            return Err(AppError::new(ErrorCode::AttachmentTooLarge));
        }
        tx.execute(
            "INSERT INTO draft_attachments
                (draft_id, id, file_name, media_type, size, blob_hash, inline, content_id,
                 origin, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, ?8)",
            params![
                draft_id,
                attachment.id,
                attachment.file_name,
                attachment.media_type,
                size,
                attachment.blob.sha256,
                attachment.origin.as_str(),
                now,
            ],
        )?;
        tx.commit()?;
        self.get_draft(draft_id)
    }

    pub(crate) fn remove_attachment(
        &mut self,
        draft_id: &str,
        attachment_id: &str,
        expected_revision: u32,
        now: &str,
    ) -> Result<Draft, AppError> {
        let tx = self.db.transaction()?;
        begin_change(&tx, draft_id, expected_revision, now)?;
        let removed = tx.execute(
            "DELETE FROM draft_attachments WHERE draft_id = ?1 AND id = ?2",
            params![draft_id, attachment_id],
        )?;
        if removed == 0 {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        tx.commit()?;
        self.get_draft(draft_id)
    }
}
