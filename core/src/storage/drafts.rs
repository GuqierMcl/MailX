//! Draft rows. Content has been validated before it gets here.
use rusqlite::{OptionalExtension, Row, params};

use super::{Storage, decode, encode};
use crate::{
    AppError, ErrorCode,
    drafts::{
        types::{Draft, DraftKind, DraftSummary},
        validate::DraftContent,
    },
};

const COLUMNS: &str = "id, account_id, kind, source_email_id, to_addresses, cc_addresses,
    bcc_addresses, subject, body_text, in_reply_to, references_header, created_at,
    updated_at, revision";

/// Everything a new row needs besides the content.
pub(crate) struct NewDraft<'a> {
    pub id: &'a str,
    pub account_id: &'a str,
    pub kind: DraftKind,
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
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
    );
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
            body_text: body,
            in_reply_to: reply,
            references: refs,
            created_at: created,
            updated_at: updated,
            revision,
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

    pub(crate) fn insert_draft(&self, draft: &NewDraft<'_>) -> Result<Draft, AppError> {
        self.db.execute(
            "INSERT INTO drafts (id, account_id, kind, source_email_id, to_addresses,
                cc_addresses, bcc_addresses, subject, body_text, in_reply_to,
                references_header, created_at, updated_at, revision, client_mutation_id,
                create_canonical)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, 1, ?13, ?14)",
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
            ],
        )?;
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
                |row| Ok((parse(row)?, row.get::<_, String>(14)?)),
            )
            .optional()?;
        found
            .map(|(draft, canonical)| Ok((draft?, canonical)))
            .transpose()
    }

    pub(crate) fn get_draft(&self, id: &str) -> Result<Draft, AppError> {
        self.db
            .query_row(
                &format!("SELECT {COLUMNS} FROM drafts WHERE id = ?1"),
                [id],
                parse,
            )
            .optional()?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?
    }

    pub(crate) fn list_drafts(
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
                })
            })
            .collect()
    }

    /// Removes blank drafts that no live outbox item holds.
    pub(crate) fn purge_blank_drafts(&self) -> Result<(), AppError> {
        let mut statement = self.db.prepare(&format!("SELECT {COLUMNS} FROM drafts"))?;
        let drafts = statement
            .query_map([], parse)?
            .collect::<Result<Vec<_>, _>>()?;
        for draft in drafts {
            let draft = draft?;
            if draft.is_blank() && !self.draft_locked(&draft.id)? {
                self.db
                    .execute("DELETE FROM drafts WHERE id = ?1", [&draft.id])?;
            }
        }
        Ok(())
    }

    /// Compare-and-swap on `revision`, in one transaction.
    pub(crate) fn update_draft(
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
                updated_at = ?8, revision = ?9
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
                next
            ],
        )?;
        tx.commit()?;
        self.get_draft(id)
    }

    pub(crate) fn delete_draft(&self, id: &str) -> Result<(), AppError> {
        if self.db.execute("DELETE FROM drafts WHERE id = ?1", [id])? == 0 {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        Ok(())
    }

    /// The `Message-ID` header of a message, without angle brackets.
    pub(crate) fn message_id_header(&self, message_id: &str) -> Result<Option<String>, AppError> {
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
