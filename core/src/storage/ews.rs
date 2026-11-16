//! EWS identifiers translated into the shared, folder-local message identity.
//! Mappings survive deletions so a local uid can never be reused for another item.
use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use super::{Storage, messages::SyncTarget};
use crate::{AppError, ErrorCode};

pub(crate) const UID_VALIDITY: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EwsItem {
    pub item_id: String,
    pub change_key: Option<String>,
    pub uid: u32,
}

fn current(db: &rusqlite::Connection, target: &SyncTarget) -> Result<bool, AppError> {
    if target.uid_validity != UID_VALIDITY {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders f
         JOIN accounts a ON a.id=f.account_id
         JOIN folder_sync_cursors c ON c.folder_id=f.id
         WHERE f.id=?1 AND f.account_id=?2 AND f.mailbox_source_id=?3
           AND a.mailbox_source_id=?3 AND f.selectable=1 AND c.uid_validity=?4)",
        params![
            target.folder_id,
            target.account_id,
            target.mailbox_source_id,
            UID_VALIDITY
        ],
        |row| row.get(0),
    )?)
}

impl Storage {
    /// Allocates stable local uids only for the account's current folder generation.
    pub(crate) fn ews_assign(
        &mut self,
        target: &SyncTarget,
        items: &[(String, Option<String>)],
    ) -> Result<Option<Vec<EwsItem>>, AppError> {
        let tx = self.db.transaction()?;
        if !current(&tx, target)? {
            return Ok(None);
        }
        let mut highest: u32 = tx
            .query_row(
                "SELECT highest_uid FROM ews_folder_state WHERE folder_id=?1",
                [&target.folder_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let mut known: HashMap<String, u32> = tx.prepare(
            "SELECT item_id,uid FROM ews_items WHERE folder_id=?1 AND account_id=?2 AND mailbox_source_id=?3"
        )?.query_map(params![target.folder_id, target.account_id, target.mailbox_source_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?.collect::<Result<_, _>>()?;
        let mut assigned = Vec::with_capacity(items.len());
        for (item_id, change_key) in items {
            let uid = if let Some(&uid) = known.get(item_id) {
                tx.execute(
                    "UPDATE ews_items SET change_key=?2 WHERE folder_id=?1 AND item_id=?3",
                    params![target.folder_id, change_key, item_id],
                )?;
                uid
            } else {
                highest = highest
                    .checked_add(1)
                    .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
                tx.execute(
                    "INSERT INTO ews_items(account_id,mailbox_source_id,folder_id,item_id,change_key,uid)
                     VALUES (?1,?2,?3,?4,?5,?6)",
                    params![target.account_id, target.mailbox_source_id, target.folder_id, item_id, change_key, highest],
                )?;
                known.insert(item_id.clone(), highest);
                highest
            };
            assigned.push(EwsItem {
                item_id: item_id.clone(),
                change_key: change_key.clone(),
                uid,
            });
        }
        tx.execute(
            "INSERT INTO ews_folder_state(folder_id,highest_uid) VALUES (?1,?2)
             ON CONFLICT(folder_id) DO UPDATE SET highest_uid=excluded.highest_uid",
            params![target.folder_id, highest],
        )?;
        tx.commit()?;
        Ok(Some(assigned))
    }

    pub(crate) fn ews_item(
        &self,
        target: &SyncTarget,
        uid: u32,
    ) -> Result<Option<EwsItem>, AppError> {
        if !current(&self.db, target)? {
            return Ok(None);
        }
        Ok(self
            .db
            .query_row(
                "SELECT item_id,change_key,uid FROM ews_items
             WHERE account_id=?1 AND mailbox_source_id=?2 AND folder_id=?3 AND uid=?4",
                params![
                    target.account_id,
                    target.mailbox_source_id,
                    target.folder_id,
                    uid
                ],
                |row| {
                    Ok(EwsItem {
                        item_id: row.get(0)?,
                        change_key: row.get(1)?,
                        uid: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub(crate) fn ews_message_id(
        &self,
        target: &SyncTarget,
        uid: u32,
    ) -> Result<Option<String>, AppError> {
        if !current(&self.db, target)? {
            return Ok(None);
        }
        Ok(self.db.query_row(
            "SELECT id FROM messages WHERE account_id=?1 AND mailbox_source_id=?2 AND folder_id=?3
             AND uid_validity=?4 AND uid=?5 AND remote_state='present'",
            params![target.account_id, target.mailbox_source_id, target.folder_id, UID_VALIDITY, uid],
            |row| row.get(0),
        ).optional()?)
    }
}

#[cfg(test)]
mod tests;
