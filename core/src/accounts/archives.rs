//! The account service's side of the local archive; the rules live in
//! [`crate::storage::archives`].
use super::AccountService;
use crate::error::AppError;

impl AccountService {
    pub(crate) fn archive_messages(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.storage.archive_messages(message_ids)
    }

    pub(crate) fn unarchive_messages(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.storage.unarchive_messages(message_ids)
    }
}
