//! The account-service surface an EWS sync round needs. Everything here is
//! local: no socket, no credential lookup, no protocol-specific identity
//! leaves this module.
use super::AccountService;
use crate::error::{AppError, ErrorCode};
use crate::mail::render;
use crate::storage::ews::EwsItem;

impl AccountService {
    pub(crate) fn ews_assign(
        &mut self,
        account_id: &str,
        folder: &str,
        items: &[(String, Option<String>)],
    ) -> Result<Vec<EwsItem>, AppError> {
        self.storage.ews_assign(account_id, folder, items)
    }

    pub(crate) fn ews_item(
        &self,
        account_id: &str,
        folder: &str,
        uid: u32,
    ) -> Result<Option<(String, Option<String>)>, AppError> {
        self.storage.ews_item(account_id, folder, uid)
    }

    pub(crate) fn ews_message_id(
        &self,
        account_id: &str,
        folder_id: &str,
        uid_validity: u32,
        uid: u32,
    ) -> Result<Option<String>, AppError> {
        self.storage
            .ews_message_id(account_id, folder_id, uid_validity, uid)
    }

    pub(crate) fn ews_body_ready(&self, message_id: &str) -> Result<bool, AppError> {
        self.storage.ews_body_ready(message_id)
    }

    /// EWS fetches whole MIME messages during the sync round, so cache the
    /// body up front. On-demand body and preview readers then hit the local
    /// cache and never try to open an IMAP session for this account.
    pub(crate) fn ews_cache_body(
        &mut self,
        message_id: &str,
        raw: &[u8],
    ) -> Result<bool, AppError> {
        let identity = self.storage.body_identity(message_id)?;
        if self.storage.body_cache(&identity)?.is_some() {
            return Ok(true);
        }
        let rendered = render::render(raw, message_id)
            .map_err(|_| AppError::new(ErrorCode::StorageIncompatible))?;
        let blob = self.storage.blobs.put(raw)?;
        if !self.storage.body_begin(&identity)? {
            return Ok(false);
        }
        self.storage.body_commit(&identity, &blob, &rendered)
    }
}
