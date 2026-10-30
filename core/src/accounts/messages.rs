//! Validated mail read APIs and thin wrappers around synchronization transactions.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, SecondsFormat, Utc};

use super::{AccountService, validate_email_id, validate_uuid};
pub(crate) use crate::storage::messages::{FolderSyncCursor, SyncTarget};
use crate::{
    AppError,
    mail::{EmailDetail, EmailListQuery, EmailPage},
    storage::messages::{PageCursor, ValidatedEmailQuery},
    sync::headers::MessageRecord,
};

impl AccountService {
    pub fn list_emails(&self, query: &EmailListQuery) -> Result<EmailPage, AppError> {
        self.storage.list_emails(&validate_query(query)?)
    }

    pub fn get_email(&self, email_id: &str) -> Result<EmailDetail, AppError> {
        validate_email_id(email_id)?;
        self.storage.get_email(email_id)
    }

    pub(crate) fn sync_folders(
        &self,
        account_id: &str,
        source: &str,
    ) -> Result<Vec<(String, String)>, AppError> {
        self.storage.sync_folders(account_id, source)
    }

    pub(crate) fn update_folder_counts(
        &mut self,
        account_id: &str,
        source: &str,
        folder_id: &str,
        unread: u32,
        total: u32,
    ) -> Result<Option<usize>, AppError> {
        self.storage
            .update_folder_counts(account_id, source, folder_id, unread, total)
    }

    pub(crate) fn ensure_generation(
        &mut self,
        target: &SyncTarget,
        window_start: &str,
    ) -> Result<Option<FolderSyncCursor>, AppError> {
        self.storage.ensure_generation(target, window_start)
    }

    pub(crate) fn cached_uids(&self, target: &SyncTarget) -> Result<Vec<(u32, String)>, AppError> {
        self.storage.cached_uids(target)
    }

    pub(crate) fn store_message_batch(
        &mut self,
        target: &SyncTarget,
        records: &[MessageRecord],
        highest_uid: u32,
    ) -> Result<Option<usize>, AppError> {
        self.storage
            .store_message_batch(target, records, highest_uid)
    }

    pub(crate) fn new_arrivals(
        &self,
        target: &SyncTarget,
        uids: &[u32],
        since: &str,
    ) -> Result<Vec<crate::events::NewMailMessage>, AppError> {
        self.storage.new_arrivals(target, uids, since)
    }

    pub(crate) fn begin_backfill(
        &mut self,
        target: &SyncTarget,
        capped: bool,
    ) -> Result<Option<usize>, AppError> {
        self.storage.begin_backfill(target, capped)
    }

    pub(crate) fn finish_backfill(
        &mut self,
        target: &SyncTarget,
        capped: bool,
    ) -> Result<Option<usize>, AppError> {
        self.storage.finish_backfill(target, capped)
    }

    pub(crate) fn mark_deleted(
        &mut self,
        target: &SyncTarget,
        uids: &[u32],
    ) -> Result<Option<usize>, AppError> {
        self.storage.mark_deleted(target, uids)
    }

    pub(crate) fn apply_flags(
        &mut self,
        target: &SyncTarget,
        flags: &[(u32, Vec<String>)],
    ) -> Result<Option<usize>, AppError> {
        self.storage.apply_flags(target, flags)
    }
}

fn normalized_time(value: &str, field: &str) -> Result<String, AppError> {
    let time = DateTime::parse_from_rfc3339(value)
        .map_err(|_| AppError::validation(field))?
        .with_timezone(&Utc);
    // The persistent representation has four-digit years, so its lexical order
    // matches chronological order for every accepted query and cursor value.
    let formatted = time.to_rfc3339_opts(SecondsFormat::Millis, true);
    if formatted.len() != 24 {
        return Err(AppError::validation(field));
    }
    Ok(formatted)
}

fn validate_query(query: &EmailListQuery) -> Result<ValidatedEmailQuery, AppError> {
    let mut query = query.clone();
    if let Some(accounts) = &query.account_ids {
        for account in accounts {
            validate_uuid(account, "accountIds")?;
        }
    }
    if query.folder_id.is_some() && query.folder_role.is_some() {
        return Err(AppError::validation("folderId"));
    }
    if let Some(folder_id) = &query.folder_id {
        validate_uuid(folder_id, "folderId")?;
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(AppError::validation("limit"));
    }
    query.received_after = query
        .received_after
        .as_deref()
        .map(|value| normalized_time(value, "receivedAfter"))
        .transpose()?;
    query.received_before = query
        .received_before
        .as_deref()
        .map(|value| normalized_time(value, "receivedBefore"))
        .transpose()?;
    query.text = query
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let cursor = query
        .cursor
        .as_ref()
        .map(|value| {
            // Cursor shape is tiny; reject oversized untrusted values before allocating decoded data.
            if value.len() > 1024 {
                return Err(AppError::validation("cursor"));
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| AppError::validation("cursor"))?;
            let mut cursor: PageCursor =
                serde_json::from_slice(&bytes).map_err(|_| AppError::validation("cursor"))?;
            validate_email_id(&cursor.i).map_err(|_| AppError::validation("cursor"))?;
            cursor.r = normalized_time(&cursor.r, "cursor")?;
            Ok(cursor)
        })
        .transpose()?;
    Ok(ValidatedEmailQuery {
        query,
        cursor,
        limit,
    })
}

impl AccountService {
    pub(crate) fn set_message_flag(
        &mut self,
        message_id: &str,
        flag: crate::storage::Flag,
        wanted: bool,
    ) -> Result<crate::storage::LocalChange, AppError> {
        validate_email_id(message_id)?;
        // A local Sent copy has no server message to flag (sending.md section 9.4).
        if message_id.starts_with(crate::mail::LOCAL_SENT_PREFIX) {
            return Err(AppError::validation("emailId"));
        }
        self.storage.set_message_flag(message_id, flag, wanted)
    }

    pub(crate) fn pending_flag_changes(
        &self,
        account_id: &str,
        limit: usize,
    ) -> Result<Vec<crate::storage::PendingChange>, AppError> {
        self.storage.pending_changes(account_id, limit)
    }

    pub(crate) fn finish_flag_change(
        &mut self,
        message_id: &str,
        flag: crate::storage::Flag,
        sent: bool,
    ) -> Result<bool, AppError> {
        self.storage.finish_change(message_id, flag, sent)
    }

    pub(crate) fn retry_flag_change(
        &mut self,
        message_id: &str,
        flag: crate::storage::Flag,
    ) -> Result<(), AppError> {
        self.storage.retry_change(message_id, flag)
    }

    pub(crate) fn mark_folder_unflaggable(&mut self, folder_id: &str) -> Result<usize, AppError> {
        self.storage.mark_folder_unflaggable(folder_id)
    }

    pub(crate) fn mark_folder_flaggable(&mut self, folder_id: &str) -> Result<(), AppError> {
        self.storage.mark_folder_flaggable(folder_id)
    }
}
