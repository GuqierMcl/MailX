//! MailxCore facade for L0 `.ics` items, per-item user states, L1 context,
//! and validated `.ics` / task text generation.
use std::collections::HashMap;

use crate::{
    AppError, ErrorCode, MailxCore,
    mail::{
        action_items::{
            ActionItemExportInput, ActionItemStatus, DetectedActionItem, format_task_text,
        },
        calendar,
    },
};

impl MailxCore {
    pub async fn email_calendar_items(
        &self,
        email_id: String,
    ) -> Result<Vec<DetectedActionItem>, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || core.accounts()?.email_calendar_items(&email_id))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    pub async fn set_action_item_status(
        &self,
        email_id: String,
        item_id: String,
        status: ActionItemStatus,
    ) -> Result<(), AppError> {
        if email_id.trim().is_empty() || item_id.trim().is_empty() || item_id.len() > 128 {
            return Err(AppError::validation("actionItem.id"));
        }
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            core.accounts()?
                .action_state_set(&email_id, &item_id, status)
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    pub async fn email_action_states(
        &self,
        email_id: String,
    ) -> Result<HashMap<String, ActionItemStatus>, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || core.accounts()?.action_states_for_message(&email_id))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    pub async fn email_l1_context(
        &self,
        email_id: String,
    ) -> Result<(String, Option<String>), AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || core.accounts()?.email_l1_context(&email_id))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    /// Re-validates a user-edited action item and generates a safe RFC 5545
    /// `.ics` document (`METHOD:PUBLISH`, no attendees/alarms).
    pub fn generate_action_item_ics(input: &ActionItemExportInput) -> Result<String, AppError> {
        calendar::generate_ics(input)
    }

    /// Formats a user-edited task as clean plain text for clipboard copying.
    pub fn format_action_item_task_text(input: &ActionItemExportInput) -> Result<String, AppError> {
        let text = format_task_text(input);
        if text.trim().is_empty() {
            return Err(AppError::validation("actionItem.title"));
        }
        Ok(text)
    }
}
