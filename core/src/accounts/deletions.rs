//! The account service's side of scheduled deletions; the rules live in
//! [`crate::storage::deletions`].
use chrono::{DateTime, Duration, Utc};

use super::AccountService;
use crate::error::AppError;
use crate::storage::deletions::{PendingDeletion, ScheduledDeletion};

impl AccountService {
    pub(crate) fn schedule_deletion(
        &mut self,
        message_ids: &[String],
        confirmed_permanent: bool,
        now: DateTime<Utc>,
        delay: Duration,
    ) -> Result<ScheduledDeletion, AppError> {
        self.storage
            .schedule_deletion(message_ids, confirmed_permanent, now, delay)
    }

    pub(crate) fn cancel_deletion(&mut self, batch_id: &str) -> Result<Vec<String>, AppError> {
        self.storage.cancel_deletion(batch_id)
    }

    pub(crate) fn claim_due_deletions(
        &mut self,
        account_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(Vec<PendingDeletion>, u32), AppError> {
        self.storage.claim_due_deletions(account_id, now)
    }

    pub(crate) fn next_deletion_due(
        &self,
        account_id: &str,
    ) -> Result<Option<DateTime<Utc>>, AppError> {
        self.storage.next_deletion_due(account_id)
    }

    pub(crate) fn finish_deletions(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.storage.finish_deletions(message_ids)
    }

    pub(crate) fn deletion_attempt_failed(
        &mut self,
        message_ids: &[String],
    ) -> Result<u32, AppError> {
        self.storage.deletion_attempt_failed(message_ids)
    }

    pub(crate) fn abandon_deletions(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.storage.abandon_deletions(message_ids)
    }
}
