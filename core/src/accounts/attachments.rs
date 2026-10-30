//! Compose attachments on a draft: limits, file name cleanup, blob storage.
use super::{AccountService, new_id, now, validate_uuid};
use crate::{
    AddAttachmentRequest, AppError, AttachmentOrigin, AttachmentPolicy, Draft, ErrorCode,
    InlineImage, RemoveAttachmentRequest,
    drafts::attachments::{sanitize_file_name, sniff_media_type},
    mail::render::validate_image,
    smtp::message::new_content_id,
    storage::attachments::NewAttachment,
};

/// The image type named by the leading bytes, if it is one we send inline.
fn sniff_image_type(bytes: &[u8]) -> Result<&'static str, AppError> {
    let media_type = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return Err(AppError::new(ErrorCode::UnsupportedImage));
    };
    Ok(media_type)
}

impl AccountService {
    /// What a draft of this account may carry. The enforced limits are one
    /// default; the server's last advertised `SIZE` only adds a hint
    /// (sending.md section 16.4).
    pub fn attachment_policy(&self, account_id: &str) -> Result<AttachmentPolicy, AppError> {
        validate_uuid(account_id, "accountId")?;
        if self.storage.account_exists(account_id)? {
            Ok(AttachmentPolicy::with_server_size(
                self.storage.smtp_size_limit(account_id)?,
            ))
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
                content_id: None,
                file_name: &file_name,
                media_type: &media_type,
                blob: &blob,
                origin: AttachmentOrigin::User,
            },
            policy,
            &now(),
        )
    }

    /// Stores pictures for the body to show through `cid:`, all or nothing in
    /// one revision step. Only PNG, JPEG, GIF and WebP pass; the type comes
    /// from the bytes, never the name, and the checks for received images
    /// apply (no animated PNG, no edge above 8192 px); any other file fails
    /// the batch with `unsupportedImage`. Inline images the saved body no
    /// longer shows are dropped first, once, so a batch never removes its own
    /// members. The body is not touched: the caller inserts
    /// `<img src="cid:…">` for the returned images, in input order, and saves.
    pub fn add_inline_images(
        &mut self,
        draft_id: &str,
        expected_revision: u32,
        files: Vec<(String, Vec<u8>)>,
    ) -> Result<(Draft, Vec<InlineImage>), AppError> {
        validate_uuid(draft_id, "draftId")?;
        let draft = self.storage.get_draft(draft_id)?;
        let policy = self.attachment_policy(&draft.account_id)?;
        let total: u64 = files.iter().map(|(_, b)| b.len() as u64).sum();
        if total > policy.max_total_bytes {
            return Err(AppError::new(ErrorCode::AttachmentTooLarge));
        }
        if self.storage.draft_locked(draft_id)? {
            return Err(AppError::new(ErrorCode::DraftLocked));
        }
        let (sender, _) = self.storage.account_sender(&draft.account_id)?;
        let mut prepared = Vec::with_capacity(files.len());
        for (name, bytes) in &files {
            let file_name =
                sanitize_file_name(name).ok_or_else(|| AppError::validation("fileName"))?;
            let media_type = sniff_image_type(bytes)?;
            validate_image(bytes, media_type)
                .map_err(|_| AppError::new(ErrorCode::UnsupportedImage))?;
            let blob = self.storage.blobs.put(bytes)?;
            prepared.push((
                new_id(),
                new_content_id(&sender)?,
                file_name,
                media_type,
                blob,
            ));
        }
        let rows: Vec<NewAttachment<'_>> = prepared
            .iter()
            .map(
                |(id, content_id, file_name, media_type, blob)| NewAttachment {
                    id,
                    content_id: Some(content_id),
                    file_name,
                    media_type,
                    blob,
                    origin: AttachmentOrigin::User,
                },
            )
            .collect();
        let draft =
            self.storage
                .add_attachments(draft_id, expected_revision, &rows, policy, &now())?;
        let images = prepared
            .into_iter()
            .map(|(id, content_id, file_name, ..)| InlineImage {
                attachment_id: id,
                content_id,
                file_name,
            })
            .collect();
        Ok((draft, images))
    }

    /// The bytes the editor may display for an inline image: the validated
    /// re-encode (GIF and WebP become a still PNG), never the stored original.
    pub fn inline_image_preview(
        &self,
        draft_id: &str,
        attachment_id: &str,
    ) -> Result<(String, Vec<u8>), AppError> {
        validate_uuid(draft_id, "draftId")?;
        validate_uuid(attachment_id, "attachmentId")?;
        let file = self
            .storage
            .attachment_files(draft_id)?
            .into_iter()
            .find(|file| file.summary.id == attachment_id && file.summary.inline)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        let bytes = self.storage.blobs.read(&file.blob)?;
        let image = validate_image(&bytes, &file.summary.media_type)
            .map_err(|_| AppError::new(ErrorCode::UnsupportedImage))?;
        Ok((image.mime_type, image.bytes))
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
