//! Local EWS identity access; network I/O lives in the protocol and sync modules.
use super::AccountService;
use super::sync_plan::{EwsPlan, PlanError};
use crate::{
    AppError, ErrorCode,
    storage::{bodies::BodyIdentity, ews::EwsItem, messages::SyncTarget},
};

impl AccountService {
    pub(crate) fn ews_assign(
        &mut self,
        target: &SyncTarget,
        items: &[(String, Option<String>)],
    ) -> Result<Option<Vec<EwsItem>>, AppError> {
        self.storage.ews_assign(target, items)
    }

    pub(crate) fn ews_item(
        &self,
        target: &SyncTarget,
        uid: u32,
    ) -> Result<Option<EwsItem>, AppError> {
        self.storage.ews_item(target, uid)
    }

    pub(crate) fn ews_message_id(
        &self,
        target: &SyncTarget,
        uid: u32,
    ) -> Result<Option<String>, AppError> {
        self.storage.ews_message_id(target, uid)
    }

    /// Prepare an on-demand EWS read without letting a missing mapping fall
    /// through to IMAP. The caller retains its normal flight/identity guard.
    pub(crate) fn ews_read_plan(
        &self,
        identity: &BodyIdentity,
    ) -> Result<Option<(EwsPlan, EwsItem)>, PlanError> {
        let Some(plan) = self.ews_plan(&identity.account_id)? else {
            return Ok(None);
        };
        if plan.mailbox_source_id != identity.mailbox_source_id || !self.body_current(identity)? {
            return Err(AppError::new(ErrorCode::NotFound).into());
        }
        let target = SyncTarget {
            account_id: identity.account_id.clone(),
            mailbox_source_id: identity.mailbox_source_id.clone(),
            folder_id: identity.folder_id.clone(),
            uid_validity: identity.uid_validity,
        };
        let item = self
            .ews_item(&target, identity.uid)?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        Ok(Some((plan, item)))
    }
}
