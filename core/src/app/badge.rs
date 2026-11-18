//! The unread number a host shows on the app icon (Dock, launcher).
//!
//! It is the unified inbox's unread count the sidebar shows, taken from the
//! same folder counts: locally archived mail and mail waiting to be deleted
//! are already left out. Inboxes whose counts are not known yet are skipped,
//! so a guess never shows a number that is too low.
use crate::accounts::AccountService;
use crate::error::AppError;
use crate::mail::{Folder, FolderRole};
use crate::settings::BadgeSettings;

use super::MailxCore;

impl MailxCore {
    /// The number for the app icon badge under `settings`; 0 when the badge
    /// is off or there is no unread inbox mail. Reads only local data.
    pub fn badge_count(&self, settings: &BadgeSettings) -> Result<u32, AppError> {
        if !settings.enabled {
            return Ok(0);
        }
        badge_count_in(&*self.accounts()?, settings)
    }
}

pub(crate) fn badge_count_in(
    service: &AccountService,
    settings: &BadgeSettings,
) -> Result<u32, AppError> {
    if !settings.enabled {
        return Ok(0);
    }
    Ok(count_unread(&service.all_folders()?, settings))
}

/// Sums the unread count of every selectable inbox with known counts, outside
/// the excluded accounts.
pub(crate) fn count_unread(folders: &[Folder], settings: &BadgeSettings) -> u32 {
    if !settings.enabled {
        return 0;
    }
    folders
        .iter()
        .filter(|folder| {
            folder.selectable
                && folder.role == FolderRole::Inbox
                && folder.counts_known
                && !settings.excluded_accounts.contains(&folder.account_id)
        })
        .fold(0u32, |sum, folder| sum.saturating_add(folder.unread_count))
}

#[cfg(test)]
mod tests;
