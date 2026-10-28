//! Guarded body metadata and content-addressed raw MIME storage.
//!
//! Clone `BlobStore` before releasing the service lock. All filesystem work and
//! MIME rendering run without that lock; only the methods on `Storage` use its
//! short database transactions. Task-generation checks belong to BodyManager,
//! which must hold its generation guard across each guarded database mutation.
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Storage, decode, encode};
use crate::{
    AppError, ErrorCode,
    mail::{
        BodyFailureCode,
        render::{MAX_MESSAGE_BYTES, Rendered, RenderedPart, SANITIZER_VERSION},
    },
};

/// Durable identity captured before I/O. This type contains no credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BodyIdentity {
    pub message_id: String,
    pub account_id: String,
    pub mailbox_source_id: String,
    pub account_revision: u32,
    pub folder_id: String,
    pub remote_name: String,
    pub uid_validity: u32,
    pub uid: u32,
    pub size: Option<u64>,
}

impl BodyIdentity {
    pub(crate) fn same_message(&self, other: &Self) -> bool {
        self.message_id == other.message_id
            && self.account_id == other.account_id
            && self.mailbox_source_id == other.mailbox_source_id
            && self.account_revision == other.account_revision
            && self.folder_id == other.folder_id
            && self.remote_name == other.remote_name
            && self.uid_validity == other.uid_validity
            && self.uid == other.uid
        // SIZE is a budget hint, not identity; header sync can fill it in later.
    }
}

/// Constructed after a complete, synchronized blob write or a guarded DB read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredBlob {
    pub sha256: String,
    pub size: u64,
}

/// Internal only: HTML and raw MIME references must never become IPC DTOs.
#[derive(Clone)]
pub(crate) struct CachedBody {
    pub raw: StoredBlob,
    pub rendered: Rendered,
    pub sanitizer_version: u32,
}

#[derive(Clone)]
pub(crate) struct BlobStore {
    root: Arc<PathBuf>,
    // A clone shares this lock. It serializes short atomic file operations, not
    // network requests or MIME parsing, and is never acquired under service.
    files: Arc<Mutex<()>>,
}

fn storage_error() -> AppError {
    AppError::new(ErrorCode::StorageUnavailable)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_shard(value: &str) -> bool {
    value.len() == 2
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn owned_temporary(value: &str) -> bool {
    value
        .strip_prefix(".mailx-blob-")
        .and_then(|value| value.strip_suffix(".tmp"))
        .is_some_and(|value| {
            Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value)
        })
}

fn check_directory(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| storage_error())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(storage_error())
    }
}

fn create_directory(path: &Path) -> Result<(), AppError> {
    match fs::create_dir(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(storage_error()),
    }
    check_directory(path)
}

/// Unix supports fsync on directory handles. Windows std does not expose a
/// supported directory flush; file sync + same-volume atomic rename is used
/// there. Do not pretend a rejected Windows directory flush succeeded.
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), AppError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| storage_error())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

impl BlobStore {
    pub(crate) fn open(data_directory: &Path) -> Result<Self, AppError> {
        let root = data_directory.join("blobs");
        create_directory(&root)?;
        Ok(Self {
            root: Arc::new(root),
            files: Arc::new(Mutex::new(())),
        })
    }

    /// Put first; only after this returns may a DB transaction reference it.
    pub(crate) fn put(&self, raw: &[u8]) -> Result<StoredBlob, AppError> {
        if raw.len() > MAX_MESSAGE_BYTES {
            return Err(storage_error());
        }
        let reference = StoredBlob {
            sha256: format!("{:x}", Sha256::digest(raw)),
            size: raw.len() as u64,
        };
        let _files = self.files.lock().map_err(|_| storage_error())?;
        check_directory(&self.root)?;
        let shard = self.root.join(&reference.sha256[..2]);
        create_directory(&shard)?;
        let destination = shard.join(&reference.sha256);
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                // Never reuse a corrupt or redirected file as a ready cache.
                self.read_checked(&reference)?;
                return Ok(reference);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(storage_error()),
        }
        let temporary = shard.join(format!(".mailx-blob-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| storage_error())?;
            file.write_all(raw).map_err(|_| storage_error())?;
            file.sync_all().map_err(|_| storage_error())?;
            drop(file);
            fs::rename(&temporary, &destination).map_err(|_| storage_error())?;
            sync_directory(&shard)?;
            Ok(reference)
        })();
        if result.is_err() {
            // A failed sync/rename never reaches a ready DB transaction. If the
            // rename already happened, startup GC will remove the orphan.
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub(crate) fn read(&self, reference: &StoredBlob) -> Result<Vec<u8>, AppError> {
        let _files = self.files.lock().map_err(|_| storage_error())?;
        self.read_checked(reference)
    }

    fn read_checked(&self, reference: &StoredBlob) -> Result<Vec<u8>, AppError> {
        if !valid_hash(&reference.sha256) || reference.size > MAX_MESSAGE_BYTES as u64 {
            return Err(storage_error());
        }
        check_directory(&self.root)?;
        let shard = self.root.join(&reference.sha256[..2]);
        check_directory(&shard)?;
        let path = shard.join(&reference.sha256);
        let metadata = fs::symlink_metadata(&path).map_err(|_| storage_error())?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != reference.size
        {
            return Err(storage_error());
        }
        let mut raw = Vec::new();
        File::open(&path)
            .map_err(|_| storage_error())?
            .take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| storage_error())?;
        if raw.len() as u64 != reference.size
            || format!("{:x}", Sha256::digest(&raw)) != reference.sha256
        {
            return Err(storage_error());
        }
        Ok(raw)
    }

    /// Startup only, while Storage owns the data-directory instance lock and
    /// before workers exist. Unknown names and all symlinks are left untouched.
    pub(crate) fn cleanup(&self, referenced: &HashSet<String>) -> Result<usize, AppError> {
        let _files = self.files.lock().map_err(|_| storage_error())?;
        check_directory(&self.root)?;
        let mut removed = 0;
        for entry in fs::read_dir(&*self.root).map_err(|_| storage_error())? {
            let entry = entry.map_err(|_| storage_error())?;
            let kind = entry.file_type().map_err(|_| storage_error())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if kind.is_file() && owned_temporary(&name) {
                fs::remove_file(entry.path()).map_err(|_| storage_error())?;
                removed += 1;
                continue;
            }
            if !kind.is_dir() || kind.is_symlink() || !valid_shard(&name) {
                continue;
            }
            let shard = entry.path();
            let mut changed = false;
            for candidate in fs::read_dir(&shard).map_err(|_| storage_error())? {
                let candidate = candidate.map_err(|_| storage_error())?;
                let kind = candidate.file_type().map_err(|_| storage_error())?;
                if !kind.is_file() || kind.is_symlink() {
                    continue;
                }
                let Some(filename) = candidate.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let orphan = valid_hash(&filename)
                    && filename.starts_with(&name)
                    && !referenced.contains(&filename);
                if orphan || owned_temporary(&filename) {
                    fs::remove_file(candidate.path()).map_err(|_| storage_error())?;
                    removed += 1;
                    changed = true;
                }
            }
            if changed {
                sync_directory(&shard)?;
            }
        }
        if removed > 0 {
            sync_directory(&self.root)?;
        }
        Ok(removed)
    }
}

const BODY_IDENTITY: &str = "SELECT m.id, m.account_id, m.mailbox_source_id, a.revision,
    m.folder_id, f.remote_name, m.uid_validity, m.uid, m.size
    FROM messages m
    JOIN accounts a ON a.id = m.account_id AND a.mailbox_source_id = m.mailbox_source_id
    JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id
        AND f.mailbox_source_id = m.mailbox_source_id AND f.selectable = 1
    JOIN folder_sync_cursors c ON c.folder_id = m.folder_id AND c.uid_validity = m.uid_validity
    WHERE m.remote_state = 'present' AND m.id = ?1";

fn read_identity(db: &Connection, message_id: &str) -> Result<Option<BodyIdentity>, AppError> {
    Ok(db
        .query_row(BODY_IDENTITY, [message_id], |row| {
            Ok(BodyIdentity {
                message_id: row.get(0)?,
                account_id: row.get(1)?,
                mailbox_source_id: row.get(2)?,
                account_revision: row.get(3)?,
                folder_id: row.get(4)?,
                remote_name: row.get(5)?,
                uid_validity: row.get(6)?,
                uid: row.get(7)?,
                size: row
                    .get::<_, Option<i64>>(8)?
                    .map(|value| {
                        u64::try_from(value)
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(8, value))
                    })
                    .transpose()?,
            })
        })
        .optional()?)
}

fn unsigned<T: TryFrom<i64>>(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<T> {
    let value: i64 = row.get(column)?;
    T::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn current(db: &Connection, identity: &BodyIdentity) -> Result<bool, AppError> {
    Ok(read_identity(db, &identity.message_id)?
        .is_some_and(|current| current.same_message(identity)))
}

fn failure_value(failure: BodyFailureCode) -> &'static str {
    match failure {
        BodyFailureCode::TooLarge => "tooLarge",
        BodyFailureCode::Stale => "stale",
        BodyFailureCode::Network => "network",
        BodyFailureCode::Authentication => "authentication",
        BodyFailureCode::ProviderPolicy => "providerPolicy",
        BodyFailureCode::Protocol => "protocol",
        BodyFailureCode::Storage => "storage",
    }
}

impl Storage {
    pub(crate) fn body_identity(&self, message_id: &str) -> Result<BodyIdentity, AppError> {
        read_identity(&self.db, message_id)?.ok_or_else(|| AppError::new(ErrorCode::NotFound))
    }

    pub(crate) fn body_identity_current(&self, identity: &BodyIdentity) -> Result<bool, AppError> {
        current(&self.db, identity)
    }

    pub(crate) fn body_begin(&mut self, identity: &BodyIdentity) -> Result<bool, AppError> {
        let tx = self.db.transaction()?;
        if !current(&tx, identity)? {
            return Ok(false);
        }
        let changed = tx.execute(
            "UPDATE messages SET body_state = 'fetching', body_failure = NULL
             WHERE id = ?1 AND body_state IN ('missing', 'failed')",
            [&identity.message_id],
        )? > 0;
        tx.commit()?;
        Ok(changed)
    }

    /// Withdraw the old sanitized version before rebuilding it outside the lock.
    pub(crate) fn body_rebuild(&mut self, identity: &BodyIdentity) -> Result<bool, AppError> {
        let tx = self.db.transaction()?;
        if !current(&tx, identity)? {
            return Ok(false);
        }
        let changed = tx.execute(
            "UPDATE messages SET body_state = 'fetching', body_failure = NULL
             WHERE id = ?1 AND body_state = 'ready'",
            [&identity.message_id],
        )? > 0;
        tx.commit()?;
        Ok(changed)
    }

    /// Call only under the manager's matching task-generation guard.
    pub(crate) fn body_fail(
        &mut self,
        identity: &BodyIdentity,
        failure: BodyFailureCode,
    ) -> Result<bool, AppError> {
        let tx = self.db.transaction()?;
        if !current(&tx, identity)? {
            return Ok(false);
        }
        let changed = tx.execute(
            "UPDATE messages SET body_state = 'failed', body_failure = ?2
             WHERE id = ?1 AND body_state = 'fetching'",
            params![identity.message_id, failure_value(failure)],
        )? > 0;
        tx.commit()?;
        Ok(changed)
    }

    /// A cancelled old flight cannot overwrite a ready result. The manager must
    /// additionally exclude a newer flight under its task-generation guard.
    pub(crate) fn body_reset(&mut self, identity: &BodyIdentity) -> Result<bool, AppError> {
        let tx = self.db.transaction()?;
        if !current(&tx, identity)? {
            return Ok(false);
        }
        let changed = tx.execute(
            "UPDATE messages SET body_state = 'missing', body_failure = NULL
             WHERE id = ?1 AND body_state = 'fetching'",
            [&identity.message_id],
        )? > 0;
        tx.commit()?;
        Ok(changed)
    }

    pub(crate) fn body_cache(
        &self,
        identity: &BodyIdentity,
    ) -> Result<Option<CachedBody>, AppError> {
        if !current(&self.db, identity)? {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        let result = self
            .db
            .query_row(
                "SELECT b.raw_sha256, b.raw_size, b.plain_text, b.sanitized_html,
                b.sanitizer_version, b.links, b.remote_images_blocked, b.truncated
             FROM message_bodies b JOIN messages m ON m.id = b.message_id
             WHERE b.message_id = ?1 AND m.body_state = 'ready'",
                [&identity.message_id],
                |row| {
                    Ok((
                        StoredBlob {
                            sha256: row.get(0)?,
                            size: unsigned(row, 1)?,
                        },
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, u32>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, u32>(6)?,
                        row.get::<_, bool>(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            raw,
            plain_text,
            sanitized_html,
            sanitizer_version,
            links,
            remote_images_blocked,
            truncated,
        )) = result
        else {
            return Ok(None);
        };
        if !valid_hash(&raw.sha256) || raw.size > MAX_MESSAGE_BYTES as u64 {
            return Err(AppError::new(ErrorCode::StorageIncompatible));
        }
        let mut statement = self.db.prepare(
            "SELECT part_index, content_id, filename, mime_type, size, is_inline
             FROM message_parts WHERE message_id = ?1 ORDER BY part_index",
        )?;
        let parts = statement
            .query_map([&identity.message_id], |row| {
                Ok(RenderedPart {
                    part_index: unsigned(row, 0)?,
                    content_id: row.get(1)?,
                    filename: row.get(2)?,
                    mime_type: row.get(3)?,
                    size: unsigned(row, 4)?,
                    is_inline: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(CachedBody {
            raw,
            rendered: Rendered {
                plain_text,
                sanitized_html,
                links: decode(&links)?,
                remote_images_blocked,
                truncated,
                parts,
            },
            sanitizer_version,
        }))
    }

    /// Resource paths need only the raw reference, never a copy of the HTML.
    pub(crate) fn body_raw(&self, identity: &BodyIdentity) -> Result<Option<StoredBlob>, AppError> {
        Ok(self.body_stamp(identity)?.map(|(raw, _version)| raw))
    }

    pub(crate) fn body_stamp(
        &self,
        identity: &BodyIdentity,
    ) -> Result<Option<(StoredBlob, u32)>, AppError> {
        if !current(&self.db, identity)? {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        let raw = self
            .db
            .query_row(
                "SELECT b.raw_sha256, b.raw_size, b.sanitizer_version FROM message_bodies b
             JOIN messages m ON m.id = b.message_id
             WHERE b.message_id = ?1 AND m.body_state = 'ready'",
                [&identity.message_id],
                |row| {
                    Ok((
                        StoredBlob {
                            sha256: row.get(0)?,
                            size: unsigned(row, 1)?,
                        },
                        row.get(2)?,
                    ))
                },
            )
            .optional()?;
        if raw.as_ref().is_some_and(|(raw, _version)| {
            !valid_hash(&raw.sha256) || raw.size > MAX_MESSAGE_BYTES as u64
        }) {
            return Err(AppError::new(ErrorCode::StorageIncompatible));
        }
        Ok(raw)
    }

    /// Duplicate CIDs resolve to the first inline entity in MIME pre-order.
    /// The selected entity is always scoped to this currently visible message.
    /// The raw message and the stored description of one listed attachment.
    pub(crate) fn body_attachment(
        &self,
        identity: &BodyIdentity,
        part_index: usize,
    ) -> Result<Option<(StoredBlob, RenderedPart)>, AppError> {
        let Some(raw) = self.body_raw(identity)? else {
            return Ok(None);
        };
        let part = self
            .db
            .query_row(
                "SELECT part_index, content_id, filename, mime_type, size, is_inline
                 FROM message_parts WHERE message_id = ?1 AND part_index = ?2 AND is_inline = 0",
                params![identity.message_id, part_index as i64],
                |row| {
                    Ok(RenderedPart {
                        part_index: row.get::<_, i64>(0)? as usize,
                        content_id: row.get(1)?,
                        filename: row.get(2)?,
                        mime_type: row.get(3)?,
                        size: row.get::<_, i64>(4)?.max(0) as u64,
                        is_inline: false,
                    })
                },
            )
            .optional()
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
        Ok(part.map(|part| (raw, part)))
    }

    pub(crate) fn body_part(
        &self,
        identity: &BodyIdentity,
        content_id: &str,
    ) -> Result<Option<(StoredBlob, RenderedPart)>, AppError> {
        let Some(raw) = self.body_raw(identity)? else {
            return Ok(None);
        };
        let part = self
            .db
            .query_row(
                "SELECT part_index, content_id, filename, mime_type, size, is_inline
             FROM message_parts WHERE message_id = ?1 AND content_id = ?2 AND is_inline = 1
             ORDER BY part_index LIMIT 1",
                params![identity.message_id, content_id],
                |row| {
                    Ok(RenderedPart {
                        part_index: unsigned(row, 0)?,
                        content_id: row.get(1)?,
                        filename: row.get(2)?,
                        mime_type: row.get(3)?,
                        size: unsigned(row, 4)?,
                        is_inline: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(part.map(|part| (raw, part)))
    }

    pub(crate) fn body_link(
        &self,
        identity: &BodyIdentity,
        index: usize,
    ) -> Result<Option<String>, AppError> {
        Ok(self
            .body_links(identity)?
            .and_then(|links| links.get(index).cloned()))
    }

    /// Link enumeration never selects cached HTML or MIME payloads.
    pub(crate) fn body_links(
        &self,
        identity: &BodyIdentity,
    ) -> Result<Option<Vec<String>>, AppError> {
        if !current(&self.db, identity)? {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        let links = self
            .db
            .query_row(
                "SELECT b.links FROM message_bodies b JOIN messages m ON m.id = b.message_id
             WHERE b.message_id = ?1 AND m.body_state = 'ready'",
                [&identity.message_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        links.map(|links| decode(&links)).transpose()
    }

    /// `blob` has already been durably written outside the service lock. A failed
    /// identity check or transaction leaves only an unreferenced startup-GC blob.
    pub(crate) fn body_commit(
        &mut self,
        identity: &BodyIdentity,
        blob: &StoredBlob,
        rendered: &Rendered,
    ) -> Result<bool, AppError> {
        if !valid_hash(&blob.sha256) || blob.size > MAX_MESSAGE_BYTES as u64 {
            return Err(storage_error());
        }
        let links = encode(&rendered.links)?;
        let preview: String = rendered
            .plain_text
            .as_deref()
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        let tx = self.db.transaction()?;
        if !current(&tx, identity)? {
            return Ok(false);
        }
        let eligible: bool = tx.query_row(
            "SELECT body_state IN ('fetching', 'ready') FROM messages WHERE id = ?1",
            [&identity.message_id],
            |row| row.get(0),
        )?;
        if !eligible {
            return Ok(false);
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        tx.execute(
            "INSERT INTO message_bodies
                (message_id, raw_sha256, raw_size, plain_text, sanitized_html,
                 sanitizer_version, links, remote_images_blocked, truncated, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(message_id) DO UPDATE SET
                raw_sha256=excluded.raw_sha256, raw_size=excluded.raw_size,
                plain_text=excluded.plain_text, sanitized_html=excluded.sanitized_html,
                sanitizer_version=excluded.sanitizer_version, links=excluded.links,
                remote_images_blocked=excluded.remote_images_blocked,
                truncated=excluded.truncated",
            params![
                identity.message_id,
                blob.sha256,
                i64::try_from(blob.size).map_err(|_| storage_error())?,
                rendered.plain_text,
                rendered.sanitized_html,
                SANITIZER_VERSION,
                links,
                rendered.remote_images_blocked,
                rendered.truncated,
                now
            ],
        )?;
        tx.execute(
            "DELETE FROM message_parts WHERE message_id = ?1",
            [&identity.message_id],
        )?;
        {
            let mut statement = tx.prepare(
                "INSERT INTO message_parts (message_id, part_index, content_id, filename, mime_type, size, is_inline)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for part in &rendered.parts {
                statement.execute(params![
                    identity.message_id,
                    i64::try_from(part.part_index).map_err(|_| storage_error())?,
                    part.content_id,
                    part.filename,
                    part.mime_type,
                    i64::try_from(part.size).map_err(|_| storage_error())?,
                    part.is_inline
                ])?;
            }
        }
        // The header-based guess becomes the real answer: any listed non-inline part.
        let has_attachment = rendered.parts.iter().any(|part| !part.is_inline);
        tx.execute(
            "UPDATE messages SET preview = ?2, body_state = 'ready', body_failure = NULL, has_attachment = ?3 WHERE id = ?1",
            params![identity.message_id, preview, has_attachment],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Called exactly once by open(), before any service/worker exists.
    pub(crate) fn recover_bodies(&mut self) -> Result<(), AppError> {
        self.db.execute(
            "UPDATE messages SET body_state = 'missing', body_failure = NULL WHERE body_state = 'fetching'",
            [],
        )?;
        let referenced = {
            let mut statement = self
                .db
                .prepare("SELECT DISTINCT raw_sha256 FROM message_bodies")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()?
        };
        self.blobs.cleanup(&referenced)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
