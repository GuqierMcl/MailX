//! Compose attachments on a draft: limits, file name cleanup, blob storage.
use super::{AccountService, new_id, now, validate_uuid};
use crate::{
    AddAttachmentRequest, AppError, AttachmentOrigin, AttachmentPolicy, Draft, ErrorCode,
    RemoveAttachmentRequest,
    drafts::attachments::{sanitize_file_name, sniff_media_type},
    storage::attachments::NewAttachment,
};

impl AccountService {
    /// What a draft of this account may carry. One default now; provider
    /// metadata overrides it later (sending.md section 16.4).
    pub fn attachment_policy(&self, account_id: &str) -> Result<AttachmentPolicy, AppError> {
        validate_uuid(account_id, "accountId")?;
        if self.storage.account_exists(account_id)? {
            Ok(AttachmentPolicy::default())
        } else {
            Err(AppError::new(ErrorCode::NotFound))
        }
    }

    /// Stores `bytes` as a user-added attachment. Identical content reuses the
    /// blob already on disk; the media type is sniffed, never a reason to refuse.
    pub fn add_attachment(
        &mut self,
        request: AddAttachmentRequest,
        bytes: &[u8],
    ) -> Result<Draft, AppError> {
        validate_uuid(&request.draft_id, "draftId")?;
        let file_name = sanitize_file_name(&request.file_name)
            .ok_or_else(|| AppError::validation("fileName"))?;
        let draft = self.storage.get_draft(&request.draft_id)?;
        let policy = self.attachment_policy(&draft.account_id)?;
        // Fail the obvious cases before writing a large file.
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > policy.max_total_bytes {
            return Err(AppError::new(ErrorCode::AttachmentTooLarge));
        }
        if self.storage.draft_locked(&request.draft_id)? {
            return Err(AppError::new(ErrorCode::DraftLocked));
        }
        let media_type = sniff_media_type(bytes, &file_name);
        let blob = self.storage.blobs.put(bytes)?;
        let id = new_id();
        self.storage.add_attachment(
            &request.draft_id,
            request.expected_revision,
            &NewAttachment {
                id: &id,
                file_name: &file_name,
                media_type: &media_type,
                blob: &blob,
                origin: AttachmentOrigin::User,
            },
            policy,
            &now(),
        )
    }

    pub fn remove_attachment(
        &mut self,
        request: RemoveAttachmentRequest,
    ) -> Result<Draft, AppError> {
        validate_uuid(&request.draft_id, "draftId")?;
        validate_uuid(&request.attachment_id, "attachmentId")?;
        self.storage.remove_attachment(
            &request.draft_id,
            &request.attachment_id,
            request.expected_revision,
            &now(),
        )
    }
}
