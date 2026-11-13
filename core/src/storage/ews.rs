//! EWS has no IMAP UIDs. These tables translate opaque `ItemId`s into the
//! stable `(folder_id, uid)` identity every shared message, search, body and
//! flag path already understands, so nothing above this module learns EWS.
use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use super::Storage;
use crate::{AppError, ErrorCode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EwsItem {
    pub item_id: String,
    pub change_key: Option<String>,
    pub uid: u32,
}

impl Storage {
    /// Returns a stable uid for every ItemId in one folder, allocating new
    /// uids for items seen for the first time. `folder` is the EWS FolderId.
    pub(crate) fn ews_assign(
        &mut self,
        account_id: &str,
        folder: &str,
        items: &[(String, Option<String>)],
    ) -> Result<Vec<EwsItem>, AppError> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let tx = self.db.transaction()?;
        let mut highest: u32 = tx
            .query_row(
                "SELECT highest_uid FROM ews_folder_state WHERE account_id=?1 AND folder_id=?2",
                params![account_id, folder],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let known: HashMap<String, u32> = tx
            .prepare("SELECT item_id, uid FROM ews_items WHERE account_id=?1 AND folder_id=?2")?
            .query_map(params![account_id, folder], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })?
            .collect::<Result<_, _>>()?;
        let mut assigned = Vec::with_capacity(items.len());
        for (item_id, change_key) in items {
            let uid = match known.get(item_id) {
                Some(uid) => *uid,
                None => {
                    highest = highest
                        .checked_add(1)
                        .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
                    tx.execute(
                        "INSERT INTO ews_items(account_id,folder_id,item_id,change_key,uid) VALUES (?1,?2,?3,?4,?5)",
                        params![account_id, folder, item_id, change_key, highest],
                    )?;
                    highest
                }
            };
            if known.contains_key(item_id) {
                tx.execute(
                    "UPDATE ews_items SET change_key=?4 WHERE account_id=?1 AND folder_id=?2 AND item_id=?3",
                    params![account_id, folder, item_id, change_key],
                )?;
            }
            assigned.push(EwsItem {
                item_id: item_id.clone(),
                change_key: change_key.clone(),
                uid,
            });
        }
        tx.execute(
            "INSERT INTO ews_folder_state(account_id,folder_id,highest_uid,last_synced_at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(account_id,folder_id) DO UPDATE SET highest_uid=excluded.highest_uid, last_synced_at=excluded.last_synced_at",
            params![account_id, folder, highest, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(assigned)
    }

    /// Reverse lookup used by on-demand reads and flag write-back.
    pub(crate) fn ews_item(
        &self,
        account_id: &str,
        folder: &str,
        uid: u32,
    ) -> Result<Option<(String, Option<String>)>, AppError> {
        Ok(self
            .db
            .query_row(
                "SELECT item_id, change_key FROM ews_items WHERE account_id=?1 AND folder_id=?2 AND uid=?3",
                params![account_id, folder, uid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Whether a whole MIME message has already been rendered and cached.
    pub(crate) fn ews_body_ready(&self, message_id: &str) -> Result<bool, AppError> {
        Ok(self
            .db
            .query_row(
                "SELECT body_state = 'ready' FROM messages WHERE id = ?1",
                [message_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// The local message id a stored uid belongs to, for body pre-caching.
    pub(crate) fn ews_message_id(
        &self,
        account_id: &str,
        folder_id: &str,
        uid_validity: u32,
        uid: u32,
    ) -> Result<Option<String>, AppError> {
        Ok(self
            .db
            .query_row(
                "SELECT id FROM messages WHERE account_id=?1 AND folder_id=?2 AND uid_validity=?3 AND uid=?4 AND remote_state='present'",
                params![account_id, folder_id, uid_validity, uid],
                |row| row.get(0),
            )
            .optional()?)
    }
}
