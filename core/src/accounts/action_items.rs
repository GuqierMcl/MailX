//! AccountService methods for action-item storage, L0 `.ics` extraction, and
//! L1 context lookups.
use std::collections::HashMap;

use crate::{
    AppError, ErrorCode,
    accounts::AccountService,
    mail::{
        action_items::{ActionItemStatus, DetectedActionItem},
        calendar,
    },
    storage::action_items::{ActionExtractionKey, StoredActionExtraction},
};

impl AccountService {
    pub(crate) fn action_extraction_put(
        &mut self,
        key: &ActionExtractionKey<'_>,
        items: &[DetectedActionItem],
    ) -> Result<String, AppError> {
        self.storage.action_extraction_put(key, items)
    }

    pub(crate) fn action_extraction_get(
        &self,
        key: &ActionExtractionKey<'_>,
    ) -> Result<Option<StoredActionExtraction>, AppError> {
        self.storage.action_extraction_get(key)
    }

    pub(crate) fn action_state_set(
        &mut self,
        message_id: &str,
        item_key: &str,
        status: ActionItemStatus,
    ) -> Result<(), AppError> {
        // Verify message exists and is visible.
        let _ = self.storage.body_identity(message_id)?;
        self.storage.action_state_set(message_id, item_key, status)
    }

    pub(crate) fn action_states_for_message(
        &self,
        message_id: &str,
    ) -> Result<HashMap<String, ActionItemStatus>, AppError> {
        self.storage.action_states_for_message(message_id)
    }

    pub(crate) fn action_state_clear_ai_dismissed(
        &mut self,
        message_id: &str,
    ) -> Result<(), AppError> {
        self.storage.action_state_clear_ai_dismissed(message_id)
    }

    /// Reads L0 `.ics` events from the locally cached raw MIME of `email_id`,
    /// merged with persisted `added` / `dismissed` user states.
    pub(crate) fn email_calendar_items(
        &self,
        email_id: &str,
    ) -> Result<Vec<DetectedActionItem>, AppError> {
        let identity = self.storage.body_identity(email_id)?;
        let Some(blob) = self.storage.body_raw(&identity)? else {
            return Ok(Vec::new());
        };
        let raw = self.storage.blobs.read(&blob)?;
        let mut items = calendar::extract_ics_from_eml(&raw);
        if items.is_empty() {
            return Ok(items);
        }
        let states = self.storage.action_states_for_message(email_id)?;
        for item in &mut items {
            if let Some(status) = states.get(&item.id) {
                item.status = *status;
            }
        }
        Ok(items)
    }

    /// Returns the sanitized link list of `email_id` (used to whitelist URLs
    /// returned by L2 AI extraction).
    pub(crate) fn email_sanitized_links(&self, email_id: &str) -> Result<Vec<String>, AppError> {
        let identity = self.storage.body_identity(email_id)?;
        Ok(self.storage.body_links(&identity)?.unwrap_or_default())
    }

    /// Returns `(subject, received_at)` for L1 rule detection when rendering a
    /// received email document.
    pub(crate) fn email_l1_context(
        &self,
        email_id: &str,
    ) -> Result<(String, Option<String>), AppError> {
        use rusqlite::OptionalExtension;
        let row: Option<(String, String)> = self
            .storage
            .db
            .query_row(
                "SELECT subject, received_at FROM messages WHERE id = ?1",
                [email_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (subject, received_at) = row.ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        Ok((subject, Some(received_at)))
    }
}
