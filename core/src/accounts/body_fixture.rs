//! Synthetic profile construction, compiled only for tests and the WebView probe.
use rusqlite::params;
use uuid::Uuid;

use super::AccountService;
use crate::{AppError, ErrorCode, storage::bodies::BodyIdentity};

impl AccountService {
    /// No auth profile or endpoint is created: cached-only tests have no usable
    /// credentials or connection plan, and no sync worker is started for this row.
    pub(crate) fn body_fixture_seed(
        &mut self,
        sizes: &[usize],
    ) -> Result<Vec<BodyIdentity>, AppError> {
        let account = Uuid::new_v4().to_string();
        let source = Uuid::new_v4().to_string();
        let folder = Uuid::new_v4().to_string();
        let ids: Vec<_> = sizes.iter().map(|_| Uuid::new_v4().to_string()).collect();
        let timestamp = "2026-10-19T00:00:00.000Z";
        let tx = self.storage.db.transaction()?;
        tx.execute(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES (?1, 1, 'Synthetic body fixture', 'fixture@example.invalid', ?2, ?3, ?3)",
            params![account, source, timestamp],
        )?;
        tx.execute(
            "INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role,
             selectable, attributes, unread_count, total_count, synced_at)
             VALUES (?1, ?2, ?3, 'INBOX', 'Synthetic inbox', 'inbox', 1, '[]', ?4, ?4, ?5)",
            params![
                folder,
                account,
                source,
                i64::try_from(sizes.len()).map_err(|_| AppError::new(ErrorCode::Validation))?,
                timestamp
            ],
        )?;
        tx.execute(
            "INSERT INTO folder_sync_cursors(folder_id, uid_validity, window_start, window_capped,
             backfill_done, highest_uid, updated_at) VALUES (?1, 17, '2026-09-19', 0, 1, ?2, ?3)",
            params![
                folder,
                i64::try_from(sizes.len()).map_err(|_| AppError::new(ErrorCode::Validation))?,
                timestamp
            ],
        )?;
        for (position, (id, size)) in ids.iter().zip(sizes).enumerate() {
            let uid =
                u32::try_from(position + 1).map_err(|_| AppError::new(ErrorCode::Validation))?;
            let size = i64::try_from(*size).map_err(|_| AppError::new(ErrorCode::Validation))?;
            tx.execute(
                "INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity,
                 uid, subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
                 received_at, size, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 17, ?5, 'Synthetic fixture', 'fixture@example.invalid',
                 '[]', '[]', '[]', ?6, ?7, 0, 0, '[]', 1, 'present', ?6)",
                params![id, account, source, folder, uid, timestamp, size],
            )?;
        }
        tx.commit()?;
        ids.iter()
            .map(|id| self.storage.body_identity(id))
            .collect()
    }
}
