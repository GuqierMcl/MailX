//! Folder list cache. Rows are scoped to the account's current mailbox source,
//! so a server change never shows folders that belong to the old server.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::{Storage, encode};
use crate::error::AppError;
use crate::mail::{Folder, FolderRole};
use crate::sync::folders::FolderRecord;

fn role_text(role: FolderRole) -> &'static str {
    match role {
        FolderRole::Inbox => "inbox",
        FolderRole::Sent => "sent",
        FolderRole::Drafts => "drafts",
        FolderRole::Archive => "archive",
        FolderRole::Junk => "junk",
        FolderRole::Trash => "trash",
        FolderRole::Other => "other",
    }
}

fn role_from_text(text: &str) -> FolderRole {
    match text {
        "inbox" => FolderRole::Inbox,
        "sent" => FolderRole::Sent,
        "drafts" => FolderRole::Drafts,
        "archive" => FolderRole::Archive,
        "junk" => FolderRole::Junk,
        "trash" => FolderRole::Trash,
        _ => FolderRole::Other,
    }
}

impl Storage {
    /// Replaces an account's folder list with a fresh server listing.
    ///
    /// Folders keep their local id across syncs when the server name is
    /// unchanged. Returns `false` and writes nothing when the account is gone
    /// or now points at a different server than the listing came from.
    pub fn replace_folders(
        &mut self,
        account_id: &str,
        mailbox_source_id: &str,
        records: &[FolderRecord],
        synced_at: &str,
    ) -> Result<bool, AppError> {
        let tx = self.db.transaction()?;
        let current: Option<String> = tx
            .query_row(
                "SELECT mailbox_source_id FROM accounts WHERE id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .optional()?;
        if current.as_deref() != Some(mailbox_source_id) {
            return Ok(false);
        }

        let mut ids: HashMap<String, String> = {
            let mut query = tx.prepare("SELECT remote_name, id FROM folders WHERE account_id = ?1 AND mailbox_source_id = ?2")?;
            query
                .query_map([account_id, mailbox_source_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<Result<_, _>>()?
        };
        for record in records {
            ids.entry(record.remote_name.clone())
                .or_insert_with(|| Uuid::new_v4().to_string());
        }

        tx.execute(
            "DELETE FROM folders WHERE account_id = ?1 AND mailbox_source_id <> ?2",
            [account_id, mailbox_source_id],
        )?;
        {
            let mut existing = tx.prepare(
                "SELECT remote_name FROM folders WHERE account_id = ?1 AND mailbox_source_id = ?2",
            )?;
            let stale: Vec<String> = existing
                .query_map([account_id, mailbox_source_id], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?
                .into_iter()
                .filter(|name| !records.iter().any(|record| &record.remote_name == name))
                .collect();
            for name in stale {
                tx.execute(
                    "DELETE FROM folders WHERE account_id = ?1 AND mailbox_source_id = ?2 AND remote_name = ?3",
                    params![account_id, mailbox_source_id, name],
                )?;
            }
        }

        let mut upsert = tx.prepare(
            "INSERT INTO folders (id, account_id, mailbox_source_id, remote_name, parent_id, name, delimiter, role, selectable, attributes, unread_count, total_count, uid_next, uid_validity, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT (account_id, mailbox_source_id, remote_name) DO UPDATE SET
               parent_id = excluded.parent_id, name = excluded.name, delimiter = excluded.delimiter, role = excluded.role,
               selectable = excluded.selectable, attributes = excluded.attributes, unread_count = excluded.unread_count,
               total_count = excluded.total_count, uid_next = excluded.uid_next, uid_validity = excluded.uid_validity,
               synced_at = excluded.synced_at",
        )?;
        for record in records {
            let status = record.status;
            upsert.execute(params![
                ids[&record.remote_name],
                account_id,
                mailbox_source_id,
                record.remote_name,
                record
                    .parent_remote_name
                    .as_ref()
                    .map(|parent| &ids[parent]),
                record.name,
                record.delimiter,
                role_text(record.role),
                record.selectable,
                encode(&record.attributes)?,
                status.map_or(0, |status| status.unseen),
                status.map_or(0, |status| status.messages),
                status.and_then(|status| status.uid_next),
                status.and_then(|status| status.uid_validity),
                synced_at,
            ])?;
        }
        drop(upsert);
        tx.commit()?;
        Ok(true)
    }

    /// Folders cached for the account's current server, in no particular order.
    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>, AppError> {
        let mut query = self.db.prepare(
            "SELECT f.id, f.parent_id, f.name, f.role, f.selectable, f.unread_count, f.total_count
             FROM folders f JOIN accounts a ON a.id = f.account_id AND a.mailbox_source_id = f.mailbox_source_id
             WHERE f.account_id = ?1",
        )?;
        let folders = query
            .query_map([account_id], |row| {
                Ok(Folder {
                    id: row.get(0)?,
                    account_id: account_id.to_owned(),
                    parent_id: row.get(1)?,
                    name: row.get(2)?,
                    role: role_from_text(&row.get::<_, String>(3)?),
                    selectable: row.get(4)?,
                    unread_count: row.get(5)?,
                    total_count: row.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(folders)
    }
}
