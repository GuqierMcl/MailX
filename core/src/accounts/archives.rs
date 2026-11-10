//! Local-only archive operations, including precise restoration receipts.
use super::AccountService;
use crate::error::AppError;
use crate::storage::archives::{ArchiveChange, ArchiveEffects, ArchiveEntry};

impl AccountService {
    #[cfg(test)]
    pub(crate) fn archive_messages(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.storage.archive_messages(message_ids)
    }

    #[cfg(test)]
    pub(crate) fn unarchive_messages(&mut self, message_ids: &[String]) -> Result<(), AppError> {
        self.storage.unarchive_messages(message_ids)
    }

    pub(crate) fn change_archive(
        &mut self,
        ids: &[String],
        archive: bool,
        partial: bool,
    ) -> Result<ArchiveChange, AppError> {
        self.storage.change_archive(ids, archive, partial)
    }

    pub(crate) fn restore_archive(
        &mut self,
        entries: &[ArchiveEntry],
    ) -> Result<ArchiveEffects, AppError> {
        self.storage.restore_archive(entries)
    }
}
