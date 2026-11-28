//! Local persistence for L2 AI-extracted action items and per-item user states
//! (`added` / `dismissed`). Both tables cascade-delete with `messages`.
use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use super::Storage;
use crate::{
    AppError,
    mail::action_items::{ActionItemStatus, DetectedActionItem},
};

pub(crate) struct ActionExtractionKey<'a> {
    pub message_id: &'a str,
    pub source_sha256: &'a str,
    pub prompt_version: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredActionExtraction {
    pub items: Vec<DetectedActionItem>,
    pub updated_at: String,
}

impl Storage {
    pub(crate) fn action_extraction_put(
        &mut self,
        key: &ActionExtractionKey<'_>,
        items: &[DetectedActionItem],
    ) -> Result<String, AppError> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let items_json = serde_json::to_string(items).map_err(|_| AppError::new(crate::ErrorCode::Internal))?;
        self.db.execute(
            "INSERT OR REPLACE INTO message_action_extractions
                (message_id, source_sha256, prompt_version, items_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                key.message_id,
                key.source_sha256,
                key.prompt_version,
                items_json,
                now,
            ],
        )?;
        Ok(now)
    }

    pub(crate) fn action_extraction_get(
        &self,
        key: &ActionExtractionKey<'_>,
    ) -> Result<Option<StoredActionExtraction>, AppError> {
        let row: Option<(String, String)> = self
            .db
            .query_row(
                "SELECT items_json, updated_at FROM message_action_extractions
                 WHERE message_id = ?1 AND source_sha256 = ?2 AND prompt_version = ?3",
                params![key.message_id, key.source_sha256, key.prompt_version],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((items_json, updated_at)) = row else {
            return Ok(None);
        };
        let Ok(items) = serde_json::from_str::<Vec<DetectedActionItem>>(&items_json) else {
            return Ok(None);
        };
        Ok(Some(StoredActionExtraction { items, updated_at }))
    }

    pub(crate) fn action_state_set(
        &mut self,
        message_id: &str,
        item_key: &str,
        status: ActionItemStatus,
    ) -> Result<(), AppError> {
        match status {
            ActionItemStatus::Pending => {
                self.db.execute(
                    "DELETE FROM message_action_states WHERE message_id = ?1 AND item_key = ?2",
                    params![message_id, item_key],
                )?;
            }
            ActionItemStatus::Added | ActionItemStatus::Dismissed => {
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                self.db.execute(
                    "INSERT OR REPLACE INTO message_action_states
                        (message_id, item_key, status, updated_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![message_id, item_key, status.as_str(), now],
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn action_states_for_message(
        &self,
        message_id: &str,
    ) -> Result<HashMap<String, ActionItemStatus>, AppError> {
        let mut stmt = self.db.prepare(
            "SELECT item_key, status FROM message_action_states WHERE message_id = ?1",
        )?;
        let rows = stmt.query_map(params![message_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (key, status_str) = row?;
            if let Some(status) = ActionItemStatus::from_db(&status_str) {
                out.insert(key, status);
            }
        }
        Ok(out)
    }

    /// Clears `dismissed` entries for `ai:*` items on this message while keeping
    /// `added` marks intact (used on manual re-extraction).
    pub(crate) fn action_state_clear_ai_dismissed(
        &mut self,
        message_id: &str,
    ) -> Result<(), AppError> {
        self.db.execute(
            "DELETE FROM message_action_states
             WHERE message_id = ?1 AND item_key LIKE 'ai:%' AND status = 'dismissed'",
            params![message_id],
        )?;
        Ok(())
    }
}
