//! Draft attachment rows (docs/architecture/sending.md section 16.3).
//!
//! Adding or removing one is a compare-and-swap on the draft `revision`, like a
//! body edit, and is refused while the send queue holds the draft. The bytes
//! are in the blob store; a row only names the blob.
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Storage, bodies::StoredBlob, outbox::live_for_draft};
use crate::drafts::body::referenced_cids;
use crate::{
    AppError, AttachmentOrigin, AttachmentPolicy, Draft, DraftAttachmentSummary, ErrorCode,
};

const COLUMNS: &str = "id, file_name, media_type, size, blob_hash, origin, inline, content_id";

pub(crate) struct NewAttachment<'a> {
    pub id: &'a str,
    /// `Some` makes the row an inline image addressed by this Content-ID.
    pub content_id: Option<&'a str>,
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
            inline: row.get(6)?,
            content_id: row.get(7)?,
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

/// Frees the quota of inline images the saved body no longer shows. Runs only
/// when a file is added; saving a body never deletes a row, so an image the
/// user removed and then undid is still there.
fn drop_unreferenced_inline(tx: &Connection, draft_id: &str) -> Result<(), AppError> {
    let html: String = tx.query_row(
        "SELECT body_html FROM drafts WHERE id = ?1",
        [draft_id],
        |row| row.get(0),
    )?;
    let referenced = referenced_cids(&html);
    let mut statement = tx.prepare(
        "SELECT id, content_id FROM draft_attachments WHERE draft_id = ?1 AND inline = 1",
    )?;
    let stale = statement
        .query_map([draft_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(_, cid)| cid.as_ref().is_none_or(|cid| !referenced.contains(cid)))
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    for id in stale {
        tx.execute(
            "DELETE FROM draft_attachments WHERE draft_id = ?1 AND id = ?2",
            params![draft_id, id],
        )?;
    }
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
        self.add_attachments(
            draft_id,
            expected_revision,
            std::slice::from_ref(attachment),
            policy,
            now,
        )
    }

    /// Adds every row in one revision step: the stale inline images are
    /// dropped once before inserting, and the limits apply to the total after
    /// the whole batch. Any failure stores nothing.
    pub(crate) fn add_attachments(
        &mut self,
        draft_id: &str,
        expected_revision: u32,
        attachments: &[NewAttachment<'_>],
        policy: AttachmentPolicy,
        now: &str,
    ) -> Result<Draft, AppError> {
        let tx = self.db.transaction()?;
        begin_change(&tx, draft_id, expected_revision, now)?;
        drop_unreferenced_inline(&tx, draft_id)?;
        let (count, total): (i64, i64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM draft_attachments WHERE draft_id = ?1",
            [draft_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let added = i64::try_from(attachments.len()).unwrap_or(i64::MAX);
        if count.saturating_add(added) > i64::from(policy.max_count) {
            return Err(AppError::new(ErrorCode::TooManyAttachments));
        }
        let mut sizes = Vec::with_capacity(attachments.len());
        let mut sum = total;
        for attachment in attachments {
            let size = i64::try_from(attachment.blob.size)
                .map_err(|_| AppError::new(ErrorCode::AttachmentTooLarge))?;
            sum = sum.saturating_add(size);
            sizes.push(size);
        }
        if u64::try_from(sum).unwrap_or(u64::MAX) > policy.max_total_bytes {
            return Err(AppError::new(ErrorCode::AttachmentTooLarge));
        }
        for (attachment, size) in attachments.iter().zip(sizes) {
            tx.execute(
                "INSERT INTO draft_attachments
                    (draft_id, id, file_name, media_type, size, blob_hash, inline, content_id,
                     origin, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    draft_id,
                    attachment.id,
                    attachment.file_name,
                    attachment.media_type,
                    size,
                    attachment.blob.sha256,
                    attachment.content_id.is_some(),
                    attachment.content_id,
                    attachment.origin.as_str(),
                    now,
                ],
            )?;
        }
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
