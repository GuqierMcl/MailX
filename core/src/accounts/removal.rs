use super::{AccountRemoval, AccountService, RemoveAccountRequest, validate_uuid};
use crate::{AppError, ErrorCode};

impl AccountService {
    /// Commits removal before touching the host credential store. Callers must
    /// retire the account's workers and readers after this returns successfully.
    pub fn remove(&mut self, request: RemoveAccountRequest) -> Result<AccountRemoval, AppError> {
        validate_uuid(&request.account_id, "accountId")?;
        if request.expected_revision == 0 {
            return Err(AppError::validation("expectedRevision"));
        }
        // Do not call recover/get here: either can access a locked credential
        // store, and pending connection journals must be retired, never applied.
        let draft_ids = self.storage.remove_account(&request)?;
        self.cleanup_removed_secrets().ok();
        Ok(AccountRemoval {
            credentials_pending: self
                .storage
                .removed_secrets_pending(&request.account_id)
                .unwrap_or(true),
            account_id: request.account_id,
            draft_ids,
        })
    }

    pub(super) fn cleanup_removed_secrets(&self) -> Result<(), AppError> {
        let mut failed = false;
        for key in self.storage.removed_secrets()? {
            // A corrupt/shared historical reference must not delete a live
            // account's credential. Keep its intent for a later safe retry.
            if self.storage.is_referenced(&key)? {
                continue;
            }
            if self.secrets.delete(&key).is_err() {
                failed = true;
                continue;
            }
            // A crash between the host delete and this acknowledgement merely
            // retries an idempotent delete on the next open/write.
            self.storage.acknowledge_removed_secret(&key)?;
        }
        if failed {
            return Err(AppError::new(ErrorCode::SecretStoreUnavailable));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
