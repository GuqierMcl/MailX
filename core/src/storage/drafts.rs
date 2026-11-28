//! Draft rows. Content has been validated before it gets here.
use rusqlite::{OptionalExtension, Row, params};

use super::{Storage, attachments::NewAttachment, decode, encode};
use crate::{
    AppError, ErrorCode,
    drafts::{
        body::plain_text_to_html,
        types::{Draft, DraftKind, DraftOrigin, DraftSummary},
        validate::DraftContent,
    },
};

const COLUMNS: &str = "id, account_id, kind, source_email_id, to_addresses, cc_addresses,
    bcc_addresses, subject, body_text, in_reply_to, references_header, created_at,
    updated_at, revision, body_html, origin";

/// Everything a new row needs besides the content.
pub(crate) struct NewDraft<'a> {
    pub id: &'a str,
    pub account_id: &'a str,
    pub kind: DraftKind,
    pub origin: DraftOrigin,
    pub source_email_id: Option<&'a str>,
    pub content: &'a DraftContent,
    pub in_reply_to: Option<&'a str>,
    pub references: Option<&'a str>,
    pub now: &'a str,
    pub client_mutation_id: &'a str,
    pub canonical: &'a str,
}

fn parse(row: &Row<'_>) -> rusqlite::Result<Result<Draft, AppError>> {
    let (to, cc, bcc): (String, String, String) = (row.get(4)?, row.get(5)?, row.get(6)?);
    let kind: String = row.get(2)?;
    let (id, account_id, source, subject, body, reply, refs, created, updated, revision) = (
        row.get(0)?,
        row.get(1)?,
        row.get(3)?,
        row.get(7)?,
        row.get::<_, String>(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
    );
    let body_html = row
        .get::<_, Option<String>>(14)?
        .unwrap_or_else(|| plain_text_to_html(&body));
    let origin: String = row.get(15)?;
    Ok((|| {
        Ok(Draft {
            id,
            account_id,
            kind: DraftKind::parse(&kind)
                .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?,
            source_email_id: source,
            to: decode(&to)?,
            cc: decode(&cc)?,
            bcc: decode(&bcc)?,
            subject,
            body_html,
            body_text: body,
            in_reply_to: reply,
            references: refs,
            created_at: created,
            updated_at: updated,
            revision,
            attachments: Vec::new(),
            origin: DraftOrigin::parse(&origin)
                .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?,
        })
    })())
}

fn preview(body: &str) -> String {
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(120)
        .collect()
}

impl Storage {
    pub(crate) fn account_exists(&self, id: &str) -> Result<bool, AppError> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
            [id],
            |r| r.get(0),
        )?)
    }

    /// Last `SIZE` the account's SMTP server advertised, in bytes of whole
    /// MIME. `None` when nothing was learned or the server names no limit.
    pub(crate) fn smtp_size_limit(&self, id: &str) -> Result<Option<u64>, AppError> {
        let value: Option<i64> = self
            .db
            .query_row(
                "SELECT smtp_size_limit FROM accounts WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(value.and_then(|v| u64::try_from(v).ok()))
    }

    /// Remembers the advertised `SIZE`; `None` clears it (the server now names
    /// no limit). A missing account is not an error: it may have been removed
    /// while the send was in flight.
    pub(crate) fn set_smtp_size_limit(&self, id: &str, size: Option<u64>) -> Result<(), AppError> {
        let value = size.and_then(|v| i64::try_from(v).ok()).filter(|v| *v > 0);
        self.db.execute(
            "UPDATE accounts SET smtp_size_limit = ?2 WHERE id = ?1",
            rusqlite::params![id, value],
        )?;
        Ok(())
    }

    pub(crate) fn account_address(&self, id: &str) -> Result<String, AppError> {
        self.db
            .query_row(
                "SELECT email_address FROM accounts WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))
    }

    /// The draft row and its carried-over attachments in one transaction: a
    /// failure leaves neither. Attachments do not bump the revision; the draft
    /// is still the one the create request produced.
    pub(crate) fn insert_draft_with(
        &mut self,
        draft: &NewDraft<'_>,
        attachments: &[NewAttachment<'_>],
    ) -> Result<Draft, AppError> {
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO drafts (id, account_id, kind, source_email_id, to_addresses,
                cc_addresses, bcc_addresses, subject, body_text, in_reply_to,
                references_header, created_at, updated_at, revision, client_mutation_id,
                create_canonical, body_html, origin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, 1, ?13, ?14, ?15, ?16)",
            params![
                draft.id,
                draft.account_id,
                draft.kind.as_str(),
                draft.source_email_id,
                encode(&draft.content.to)?,
                encode(&draft.content.cc)?,
                encode(&draft.content.bcc)?,
                draft.content.subject,
                draft.content.body_text,
                draft.in_reply_to,
                draft.references,
                draft.now,
                draft.client_mutation_id,
                draft.canonical,
                draft.content.body_html,
                draft.origin.as_str(),
            ],
        )?;
        for attachment in attachments {
            let size = i64::try_from(attachment.blob.size)
                .map_err(|_| AppError::new(ErrorCode::AttachmentTooLarge))?;
            // An inline row is addressed by its Content-ID; the drafts the core
            // creates from a source (a forward's images) arrive this way.
            tx.execute(
                "INSERT INTO draft_attachments
                    (draft_id, id, file_name, media_type, size, blob_hash, inline, content_id,
                     origin, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    draft.id,
                    attachment.id,
                    attachment.file_name,
                    attachment.media_type,
                    size,
                    attachment.blob.sha256,
                    attachment.content_id.is_some(),
                    attachment.content_id,
                    attachment.origin.as_str(),
                    draft.now,
                ],
            )?;
        }
        tx.commit()?;
        self.get_draft(draft.id)
    }

    /// The draft a mutation id created, with the canonical form of its request.
    pub(crate) fn draft_by_mutation(
        &self,
        client_mutation_id: &str,
    ) -> Result<Option<(Draft, String)>, AppError> {
        let found = self
            .db
            .query_row(
                &format!(
                    "SELECT {COLUMNS}, create_canonical FROM drafts
                     WHERE client_mutation_id = ?1 ORDER BY created_at, id LIMIT 1"
                ),
                [client_mutation_id],
                |row| Ok((parse(row)?, row.get::<_, String>(16)?)),
            )
            .optional()?;
        found
            .map(|(draft, canonical)| Ok((self.with_attachments(draft?)?, canonical)))
            .transpose()
    }

    pub(crate) fn get_draft(&self, id: &str) -> Result<Draft, AppError> {
        let draft = self
            .db
            .query_row(
                &format!("SELECT {COLUMNS} FROM drafts WHERE id = ?1"),
                [id],
                parse,
            )
            .optional()?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))??;
        self.with_attachments(draft)
    }

    pub(crate) fn draft_list(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<DraftSummary>, AppError> {
        let mut statement = self.db.prepare(&format!(
            "SELECT {COLUMNS} FROM drafts WHERE (?1 IS NULL OR account_id = ?1)
             ORDER BY updated_at DESC, id DESC"
        ))?;
        let rows = statement
            .query_map([account_id], parse)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|draft| draft.and_then(|draft| self.with_attachments(draft)))
            .filter(|draft| !matches!(draft, Ok(draft) if draft.is_blank()))
            .map(|draft| {
                let draft = draft?;
                Ok(DraftSummary {
                    preview: preview(&draft.body_text),
                    id: draft.id,
                    account_id: draft.account_id,
                    kind: draft.kind,
                    source_email_id: draft.source_email_id,
                    to: draft.to,
                    subject: draft.subject,
                    updated_at: draft.updated_at,
                    revision: draft.revision,
                    origin: draft.origin,
                })
            })
            .collect()
    }

    /// Account and content filters run before the keyset cursor and LIMIT.
    pub(crate) fn list_drafts_page(
        &self,
        account_ids: &[String],
        origin: Option<&str>,
        query: Option<&str>,
        after: Option<(&str, &str)>,
        limit: u32,
    ) -> Result<Vec<DraftSummary>, AppError> {
        let mut collected = Vec::new();
        let target = limit as usize;
        let batch_size = limit.max(20);
        let mut cursor = after.map(|(u, i)| (u.to_owned(), i.to_owned()));
        let sql = format!(
            "SELECT {COLUMNS} FROM drafts d
            WHERE account_id IN (SELECT value FROM json_each(?1))
              AND (?2 IS NULL OR origin = ?2)
              AND (?3 IS NULL OR instr(lower(subject), lower(?3)) > 0)
              AND (?4 IS NULL OR updated_at < ?4 OR (updated_at = ?4 AND id < ?5))
            ORDER BY updated_at DESC, id DESC LIMIT ?6"
        );
        let accounts_json = encode(&account_ids)?;
        while collected.len() < target {
            let rows = self
                .db
                .prepare(&sql)?
                .query_map(
                    params![
                        &accounts_json,
                        origin,
                        query,
                        cursor.as_ref().map(|p| p.0.as_str()),
                        cursor.as_ref().map(|p| p.1.as_str()),
                        batch_size,
                    ],
                    parse,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let fetched = rows.len();
            if fetched == 0 {
                break;
            }
            for row in rows {
                let draft = self.with_attachments(row?)?;
                cursor = Some((draft.updated_at.clone(), draft.id.clone()));
                if !draft.is_blank() {
                    collected.push(DraftSummary {
                        preview: preview(&draft.body_text),
                        id: draft.id,
                        account_id: draft.account_id,
                        kind: draft.kind,
                        source_email_id: draft.source_email_id,
                        to: draft.to,
                        subject: draft.subject,
                        updated_at: draft.updated_at,
                        revision: draft.revision,
                        origin: draft.origin,
                    });
                    if collected.len() >= target {
                        break;
                    }
                }
            }
            if fetched < batch_size as usize {
                break;
            }
        }
        Ok(collected)
    }

    /// Removes blank drafts that no live outbox item holds.
    pub(crate) fn purge_blank_drafts(&self) -> Result<(), AppError> {
        let mut statement = self.db.prepare(&format!("SELECT {COLUMNS} FROM drafts"))?;
        let drafts = statement
            .query_map([], parse)?
            .collect::<Result<Vec<_>, _>>()?;
        for draft in drafts {
            let draft = self.with_attachments(draft?)?;
            if draft.is_blank() && !self.draft_locked(&draft.id)? {
                self.db
                    .execute("DELETE FROM drafts WHERE id = ?1", [&draft.id])?;
            }
        }
        Ok(())
    }

    /// Compare-and-swap on `revision`, in one transaction.
    pub(crate) fn draft_update(
        &mut self,
        id: &str,
        expected_revision: u32,
        account_id: Option<&str>,
        content: &DraftContent,
        now: &str,
    ) -> Result<Draft, AppError> {
        let tx = self.db.transaction()?;
        let current: Option<u32> = tx
            .query_row("SELECT revision FROM drafts WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        let current = current.ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        if current != expected_revision {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let next = current
            .checked_add(1)
            .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
        tx.execute(
            "UPDATE drafts SET account_id = COALESCE(?2, account_id), to_addresses = ?3,
                cc_addresses = ?4, bcc_addresses = ?5, subject = ?6, body_text = ?7,
                updated_at = ?8, revision = ?9, body_html = ?10
             WHERE id = ?1",
            params![
                id,
                account_id,
                encode(&content.to)?,
                encode(&content.cc)?,
                encode(&content.bcc)?,
                content.subject,
                content.body_text,
                now,
                next,
                content.body_html,
            ],
        )?;
        tx.commit()?;
        self.get_draft(id)
    }

    /// Replaces the draft body and updates inline attachments for a signature
    /// switch in a single compare-and-swap transaction.
    pub(crate) fn apply_draft_signature(
        &mut self,
        id: &str,
        expected_revision: u32,
        content: &DraftContent,
        added_images: &[NewAttachment<'_>],
        policy: crate::AttachmentPolicy,
        now: &str,
    ) -> Result<Draft, AppError> {
        let tx = self.db.transaction()?;
        let current: Option<u32> = tx
            .query_row("SELECT revision FROM drafts WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        let current = current.ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        if current != expected_revision {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let next = current
            .checked_add(1)
            .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
        tx.execute(
            "UPDATE drafts SET body_html = ?2, body_text = ?3, updated_at = ?4, revision = ?5
             WHERE id = ?1",
            params![id, content.body_html, content.body_text, now, next],
        )?;
        // Drop inline images no longer referenced in the updated HTML.
        let referenced = crate::drafts::body::referenced_cids(&content.body_html);
        let stale: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id, content_id FROM draft_attachments WHERE draft_id = ?1 AND inline = 1",
            )?;
            stmt.query_map([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(_, cid)| cid.as_ref().is_none_or(|cid| !referenced.contains(cid)))
            .map(|(img_id, _)| img_id)
            .collect()
        };
        for img_id in stale {
            tx.execute(
                "DELETE FROM draft_attachments WHERE draft_id = ?1 AND id = ?2",
                params![id, img_id],
            )?;
        }
        if !added_images.is_empty() {
            let (count, total): (i64, i64) = tx.query_row(
                "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM draft_attachments WHERE draft_id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let added = i64::try_from(added_images.len()).unwrap_or(i64::MAX);
            if count.saturating_add(added) > i64::from(policy.max_count) {
                return Err(AppError::new(ErrorCode::TooManyAttachments));
            }
            let mut sum = total;
            for attachment in added_images {
                let size = i64::try_from(attachment.blob.size)
                    .map_err(|_| AppError::new(ErrorCode::AttachmentTooLarge))?;
                sum = sum.saturating_add(size);
                if sum > i64::try_from(policy.max_total_bytes).unwrap_or(i64::MAX) {
                    return Err(AppError::new(ErrorCode::AttachmentTooLarge));
                }
                tx.execute(
                    "INSERT INTO draft_attachments
                        (draft_id, id, file_name, media_type, size, blob_hash, inline, content_id,
                         origin, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        id,
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
        }
        tx.commit()?;
        self.get_draft(id)
    }

    pub(crate) fn draft_delete(&self, id: &str) -> Result<(), AppError> {
        if self.db.execute("DELETE FROM drafts WHERE id = ?1", [id])? == 0 {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        Ok(())
    }

    /// The `Message-ID` header of a message, without angle brackets.
    pub(crate) fn message_id_header(&self, message_id: &str) -> Result<Option<String>, AppError> {
        if let Some(outbox_id) = message_id.strip_prefix(crate::mail::LOCAL_SENT_PREFIX) {
            return Ok(self
                .db
                .query_row(
                    "SELECT message_id FROM outbox WHERE id = ?1",
                    [outbox_id],
                    |r| r.get(0),
                )
                .optional()?);
        }
        Ok(self
            .db
            .query_row(
                "SELECT message_id_header FROM messages WHERE id = ?1",
                [message_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }
}
