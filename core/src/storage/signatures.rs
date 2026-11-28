//! Persistence for global email signatures, their inline images, and per-account defaults.
use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Storage, bodies::StoredBlob};
use crate::{
    AppError, ErrorCode,
    drafts::signatures::{
        AccountSignatureDefault, AccountSignatureMode, MAX_SIGNATURES, Signature, SignatureImage,
        SignaturesState,
    },
};

pub(crate) struct SignatureImageRow {
    pub summary: SignatureImage,
    pub blob: StoredBlob,
}

pub(crate) struct NewSignatureImage<'a> {
    pub id: &'a str,
    pub file_name: &'a str,
    pub media_type: &'a str,
    pub blob: &'a StoredBlob,
    pub content_id: &'a str,
}

pub(crate) struct SaveSignatureRow<'a> {
    pub id: &'a str,
    pub is_new: bool,
    pub name: &'a str,
    pub body_html: &'a str,
    pub body_text: &'a str,
    pub images: &'a [NewSignatureImage<'a>],
    pub now: &'a str,
}

fn parse_image(row: &Row<'_>) -> rusqlite::Result<SignatureImageRow> {
    let size = u64::try_from(row.get::<_, i64>(3)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(SignatureImageRow {
        summary: SignatureImage {
            id: row.get(0)?,
            file_name: row.get(1)?,
            media_type: row.get(2)?,
            size,
            content_id: row.get(5)?,
        },
        blob: StoredBlob {
            sha256: row.get(4)?,
            size,
        },
    })
}

fn images_for(db: &Connection, signature_id: &str) -> Result<Vec<SignatureImageRow>, AppError> {
    let mut statement = db.prepare(
        "SELECT id, file_name, media_type, size, blob_hash, content_id
         FROM signature_images WHERE signature_id = ?1 ORDER BY rowid",
    )?;
    let rows = statement
        .query_map([signature_id], parse_image)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
    Ok(rows)
}

fn all_images(db: &Connection) -> Result<HashMap<String, Vec<SignatureImage>>, AppError> {
    let mut statement = db.prepare(
        "SELECT signature_id, id, file_name, media_type, size, content_id
         FROM signature_images ORDER BY rowid",
    )?;
    let mut grouped: HashMap<String, Vec<SignatureImage>> = HashMap::new();
    let rows = statement.query_map([], |row| {
        let sig_id: String = row.get(0)?;
        let size =
            u64::try_from(row.get::<_, i64>(4)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok((
            sig_id,
            SignatureImage {
                id: row.get(1)?,
                file_name: row.get(2)?,
                media_type: row.get(3)?,
                size,
                content_id: row.get(5)?,
            },
        ))
    })?;
    for row in rows {
        let (sig_id, image) = row.map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
        grouped.entry(sig_id).or_default().push(image);
    }
    Ok(grouped)
}

impl Storage {
    pub(crate) fn signatures_state(&self) -> Result<SignaturesState, AppError> {
        let mut images = all_images(&self.db)?;
        let mut statement = self.db.prepare(
            "SELECT id, name, body_html, body_text, is_default, created_at, updated_at
             FROM signatures ORDER BY sort_order ASC, created_at ASC, id ASC",
        )?;
        let signatures = statement
            .query_map([], |row| {
                let id: String = row.get(0)?;
                Ok(Signature {
                    name: row.get(1)?,
                    body_html: row.get(2)?,
                    body_text: row.get(3)?,
                    images: Vec::new(),
                    is_default: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    id,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|mut sig| {
                sig.images = images.remove(&sig.id).unwrap_or_default();
                sig
            })
            .collect();

        let mut defaults_stmt = self.db.prepare(
            "SELECT account_id, mode, signature_id FROM account_signatures ORDER BY account_id",
        )?;
        let account_defaults = defaults_stmt
            .query_map([], |row| {
                let account_id: String = row.get(0)?;
                let mode: String = row.get(1)?;
                let sig_id: Option<String> = row.get(2)?;
                let selection = match (mode.as_str(), sig_id) {
                    ("none", None) => AccountSignatureMode::None,
                    ("specific", Some(signature_id)) => {
                        AccountSignatureMode::Specific { signature_id }
                    }
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok(AccountSignatureDefault {
                    account_id,
                    selection,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;

        Ok(SignaturesState {
            signatures,
            account_defaults,
        })
    }

    pub(crate) fn get_signature(
        &self,
        id: &str,
    ) -> Result<(Signature, Vec<SignatureImageRow>), AppError> {
        let mut sig = self
            .db
            .query_row(
                "SELECT id, name, body_html, body_text, is_default, created_at, updated_at
                 FROM signatures WHERE id = ?1",
                [id],
                |row| {
                    Ok(Signature {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        body_html: row.get(2)?,
                        body_text: row.get(3)?,
                        images: Vec::new(),
                        is_default: row.get::<_, i64>(4)? != 0,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        let rows = images_for(&self.db, id)?;
        sig.images = rows.iter().map(|row| row.summary.clone()).collect();
        Ok((sig, rows))
    }

    /// Resolves the effective default signature id for `account_id`:
    /// - `none` override -> `None`
    /// - `specific` override -> `Some(signature_id)`
    /// - no override (follow default) -> the signature marked `is_default = 1`, if any.
    pub(crate) fn account_default_signature_id(
        &self,
        account_id: &str,
    ) -> Result<Option<String>, AppError> {
        let entry: Option<(String, Option<String>)> = self
            .db
            .query_row(
                "SELECT mode, signature_id FROM account_signatures WHERE account_id = ?1",
                [account_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match entry {
            Some((mode, _)) if mode == "none" => Ok(None),
            Some((mode, Some(sig_id))) if mode == "specific" => Ok(Some(sig_id)),
            _ => Ok(self
                .db
                .query_row(
                    "SELECT id FROM signatures WHERE is_default = 1 LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?),
        }
    }

    pub(crate) fn save_signature(
        &mut self,
        row: &SaveSignatureRow<'_>,
    ) -> Result<Signature, AppError> {
        let tx = self.db.transaction()?;
        if row.is_new {
            let count: i64 = tx.query_row("SELECT COUNT(*) FROM signatures", [], |r| r.get(0))?;
            if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_SIGNATURES {
                return Err(AppError::validation("signatures.limit"));
            }
            let next_order: i64 = tx.query_row(
                "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM signatures",
                [],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO signatures (id, name, body_html, body_text, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![row.id, row.name, row.body_html, row.body_text, next_order, row.now],
            )?;
        } else {
            let updated = tx.execute(
                "UPDATE signatures SET name = ?2, body_html = ?3, body_text = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![row.id, row.name, row.body_html, row.body_text, row.now],
            )?;
            if updated == 0 {
                return Err(AppError::new(ErrorCode::NotFound));
            }
            tx.execute(
                "DELETE FROM signature_images WHERE signature_id = ?1",
                [row.id],
            )?;
        }
        for image in row.images {
            let size = i64::try_from(image.blob.size)
                .map_err(|_| AppError::new(ErrorCode::AttachmentTooLarge))?;
            tx.execute(
                "INSERT INTO signature_images
                    (signature_id, id, file_name, media_type, size, blob_hash, content_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    row.id,
                    image.id,
                    image.file_name,
                    image.media_type,
                    size,
                    image.blob.sha256,
                    image.content_id,
                    row.now,
                ],
            )?;
        }
        tx.commit()?;
        Ok(self.get_signature(row.id)?.0)
    }

    pub(crate) fn delete_signature(&mut self, id: &str) -> Result<(), AppError> {
        let deleted = self
            .db
            .execute("DELETE FROM signatures WHERE id = ?1", [id])?;
        if deleted == 0 {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        Ok(())
    }

    pub(crate) fn set_global_default_signature(
        &mut self,
        signature_id: Option<&str>,
    ) -> Result<(), AppError> {
        let tx = self.db.transaction()?;
        if let Some(sig_id) = signature_id {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM signatures WHERE id = ?1)",
                [sig_id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(AppError::new(ErrorCode::NotFound));
            }
        }
        tx.execute(
            "UPDATE signatures SET is_default = 0 WHERE is_default = 1",
            [],
        )?;
        if let Some(sig_id) = signature_id {
            tx.execute(
                "UPDATE signatures SET is_default = 1 WHERE id = ?1",
                [sig_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn set_account_default_signature(
        &mut self,
        account_id: &str,
        selection: &AccountSignatureMode,
    ) -> Result<(), AppError> {
        if !self.account_exists(account_id)? {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        match selection {
            AccountSignatureMode::FollowDefault => {
                self.db.execute(
                    "DELETE FROM account_signatures WHERE account_id = ?1",
                    [account_id],
                )?;
            }
            AccountSignatureMode::None => {
                self.db.execute(
                    "INSERT INTO account_signatures (account_id, mode, signature_id)
                     VALUES (?1, 'none', NULL)
                     ON CONFLICT(account_id) DO UPDATE SET mode = 'none', signature_id = NULL",
                    [account_id],
                )?;
            }
            AccountSignatureMode::Specific { signature_id } => {
                let exists: bool = self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM signatures WHERE id = ?1)",
                    [signature_id],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Err(AppError::new(ErrorCode::NotFound));
                }
                self.db.execute(
                    "INSERT INTO account_signatures (account_id, mode, signature_id)
                     VALUES (?1, 'specific', ?2)
                     ON CONFLICT(account_id) DO UPDATE SET mode = 'specific', signature_id = excluded.signature_id",
                    params![account_id, signature_id],
                )?;
            }
        }
        Ok(())
    }
}
