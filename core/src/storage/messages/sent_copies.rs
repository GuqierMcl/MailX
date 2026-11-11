//! Bounded repair of cached Sent identities, independent of provider names.
use super::*;

impl Storage {
    /// Revisit cached identities after visibility or folder roles change, even
    /// when no new headers are downloaded. Sync calls this only after applying
    /// known removals (or an unchanged STATUS permits skipping the full round).
    pub(crate) fn reconcile_sent_folder(
        &mut self,
        account: &str,
        source: &str,
        folder: &str,
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_folder(&tx, account, source, folder)? {
            return Ok(None);
        }
        let is_sent: bool = tx.query_row(
            "SELECT role = 'sent' FROM folders WHERE id = ?1",
            [folder],
            |row| row.get(0),
        )?;
        let matched = if is_sent {
            super::super::outbox::reconcile_sent_copies(&tx, account, Some(folder), &now())?
        } else {
            0
        };
        tx.commit()?;
        Ok(Some(matched))
    }

    /// Startup also repairs standard identities cached before reconciliation
    /// existed. Provider headers are refreshed by sync when they are missing.
    pub(crate) fn reconcile_cached_sent_copies(&mut self) -> Result<(), AppError> {
        for account in self.account_ids()? {
            let tx = self.db.transaction()?;
            super::super::outbox::reconcile_sent_copies(&tx, &account, None, &now())?;
            tx.commit()?;
        }
        Ok(())
    }

    /// Includes previously cached mail outside the normal sync window. The
    /// policy stamp makes completion durable and allows future policy upgrades.
    pub(crate) fn sent_identity_refresh_uids(
        &self,
        account: &str,
        source: &str,
        folder: &str,
        policy: &str,
    ) -> Result<Vec<u32>, AppError> {
        let mut query = self.db.prepare_cached(
            "SELECT m.uid FROM messages m
             JOIN accounts a ON a.id = m.account_id AND a.mailbox_source_id = m.mailbox_source_id
             JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id
               AND f.mailbox_source_id = m.mailbox_source_id
             JOIN folder_sync_cursors c ON c.folder_id = m.folder_id AND c.uid_validity = m.uid_validity
             WHERE m.account_id = ?1 AND m.mailbox_source_id = ?2 AND m.folder_id = ?3
               AND f.selectable = 1 AND m.remote_state = 'present'
               AND m.sent_identity_policy <> ?4
               AND NOT EXISTS (SELECT 1 FROM message_deletions d WHERE d.message_id = m.id)
             ORDER BY m.uid DESC LIMIT 200",
        )?;
        Ok(query
            .query_map(params![account, source, folder, policy], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// A successful UID FETCH is authoritative only for the requested UIDs.
    /// Refresh identity fields without overwriting local flags, previews, body
    /// caches or server dates. Never resurrect messages removed during I/O.
    pub(crate) fn store_sent_identity_batch(
        &mut self,
        target: &SyncTarget,
        records: &[MessageRecord],
        requested: &[u32],
        policy: &str,
    ) -> Result<Option<usize>, AppError> {
        let tx = self.db.transaction()?;
        if !current_target(&tx, target)? {
            return Ok(None);
        }
        let timestamp = now();
        let mut changed = 0;
        for uid in requested {
            if let Some(record) = records.iter().find(|record| record.uid == *uid) {
                if record.sent_identity_policy != policy {
                    return Err(AppError::new(ErrorCode::Internal));
                }
                tx.execute(
                    "UPDATE messages SET message_id_header = ?4, original_message_id = ?5,
                     sent_identity_policy = ?6, in_reply_to = ?7, references_header = ?8
                     WHERE folder_id = ?1 AND uid_validity = ?2 AND uid = ?3 AND remote_state = 'present'",
                    params![target.folder_id, target.uid_validity, uid, record.message_id_header,
                        record.original_message_id, policy, record.in_reply_to,
                        record.references_header],
                )?;
            } else {
                changed += tx.execute(
                    "UPDATE messages SET remote_state = 'deleted', updated_at = ?4
                     WHERE folder_id = ?1 AND uid_validity = ?2 AND uid = ?3 AND remote_state = 'present'",
                    params![target.folder_id, target.uid_validity, uid, timestamp],
                )?;
                super::super::search_index::refresh_by_uid(
                    &tx,
                    &target.folder_id,
                    target.uid_validity,
                    *uid,
                )?;
            }
        }
        changed += super::super::outbox::reconcile_sent_copies(
            &tx,
            &target.account_id,
            Some(&target.folder_id),
            &timestamp,
        )?;
        tx.commit()?;
        self.thread_stored(&target.folder_id, target.uid_validity, requested);
        Ok(Some(changed))
    }
}
