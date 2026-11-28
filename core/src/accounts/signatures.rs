//! Signature management and applying signatures to drafts.
use std::collections::{HashMap, HashSet};

use super::{AccountService, new_id, now, validate_uuid};
use crate::{
    AppError, AttachmentOrigin, Draft, ErrorCode,
    drafts::{
        attachments::sanitize_file_name,
        body::{
            apply_signature_to_html, normalize_signature, referenced_cids, rewrite_cids,
            signature_block_html,
        },
        signatures::{
            AccountSignatureMode, ApplyDraftSignatureRequest, DraftSignatureSelection,
            MAX_SIGNATURE_IMAGE_BYTES, MAX_SIGNATURE_IMAGES, MAX_SIGNATURE_NAME_BYTES,
            SaveSignatureRequest, SetAccountSignatureRequest, SetDefaultSignatureRequest,
            SignaturesState, StagedSignatureImage,
        },
        validate::{self, DraftContent},
    },
    mail::render::validate_image,
    smtp::message::new_content_id,
    storage::{
        attachments::NewAttachment,
        bodies::StoredBlob,
        signatures::{NewSignatureImage, SaveSignatureRow},
    },
};

const SIGNATURE_CID_SENDER: &str = "signature@signature.mailx.local";

/// A signature image prepared for insertion into a draft as an inline attachment.
pub(crate) struct PreparedSignatureImage {
    pub id: String,
    pub content_id: String,
    pub file_name: String,
    pub media_type: String,
    pub blob: StoredBlob,
}

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

fn clean_signature_name(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_SIGNATURE_NAME_BYTES
        || trimmed.chars().any(char::is_control)
    {
        return Err(AppError::validation("signature.name"));
    }
    Ok(trimmed.to_owned())
}

impl AccountService {
    pub fn signatures_state(&self) -> Result<SignaturesState, AppError> {
        self.storage.signatures_state()
    }

    /// Validates an image for a signature editor, stores its raw bytes in the
    /// blob store, and returns staged metadata alongside safe re-encoded preview bytes.
    pub fn prepare_signature_image(
        &self,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<(StagedSignatureImage, String, Vec<u8>), AppError> {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SIGNATURE_IMAGE_BYTES {
            return Err(AppError::new(ErrorCode::AttachmentTooLarge));
        }
        let file_name =
            sanitize_file_name(file_name).ok_or_else(|| AppError::validation("fileName"))?;
        let media_type = sniff_image_type(bytes)?;
        let validated = validate_image(bytes, media_type)
            .map_err(|_| AppError::new(ErrorCode::UnsupportedImage))?;
        let blob = self.storage.blobs.put(bytes)?;
        let staged = StagedSignatureImage {
            id: new_id(),
            file_name,
            media_type: media_type.to_owned(),
            size: blob.size,
            content_id: new_content_id(SIGNATURE_CID_SENDER)?,
            blob_hash: blob.sha256,
        };
        Ok((staged, validated.mime_type, validated.bytes))
    }

    /// Returns safe re-encoded preview bytes for an inline image on a saved signature.
    pub fn signature_image_preview(
        &self,
        signature_id: &str,
        image_id: &str,
    ) -> Result<(String, Vec<u8>), AppError> {
        validate_uuid(signature_id, "signatureId")?;
        validate_uuid(image_id, "imageId")?;
        let (_, images) = self.storage.get_signature(signature_id)?;
        let row = images
            .into_iter()
            .find(|item| item.summary.id == image_id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        let bytes = self.storage.blobs.read(&row.blob)?;
        let validated = validate_image(&bytes, &row.summary.media_type)
            .map_err(|_| AppError::new(ErrorCode::UnsupportedImage))?;
        Ok((validated.mime_type, validated.bytes))
    }

    pub fn save_signature(
        &mut self,
        request: SaveSignatureRequest,
    ) -> Result<SignaturesState, AppError> {
        let is_new = request.id.is_none();
        let id = match request.id {
            Some(existing) => {
                validate_uuid(&existing, "signature.id")?;
                existing
            }
            None => new_id(),
        };
        let name = clean_signature_name(&request.name)?;
        let body = normalize_signature(&request.body_html)?;
        let referenced: HashSet<String> = referenced_cids(&body.html).into_iter().collect();

        // Collect candidate images: existing images (if editing) + newly staged images.
        let mut by_cid: HashMap<String, (String, String, String, StoredBlob)> = HashMap::new();
        if !is_new {
            let (_, existing_images) = self.storage.get_signature(&id)?;
            for row in existing_images {
                by_cid.insert(
                    row.summary.content_id,
                    (
                        row.summary.id,
                        row.summary.file_name,
                        row.summary.media_type,
                        row.blob,
                    ),
                );
            }
        }
        for staged in request.staged_images {
            if !referenced.contains(&staged.content_id) {
                continue;
            }
            validate_uuid(&staged.id, "stagedImage.id")?;
            let file_name = sanitize_file_name(&staged.file_name)
                .ok_or_else(|| AppError::validation("stagedImage.fileName"))?;
            let blob = StoredBlob {
                sha256: staged.blob_hash,
                size: staged.size,
            };
            // Verify the blob exists and is a valid image before recording it.
            let raw = self.storage.blobs.read(&blob)?;
            let media_type = sniff_image_type(&raw)?;
            validate_image(&raw, media_type)
                .map_err(|_| AppError::new(ErrorCode::UnsupportedImage))?;
            by_cid.insert(
                staged.content_id,
                (staged.id, file_name, media_type.to_owned(), blob),
            );
        }

        // Keep only images actually referenced in the normalized body, in order.
        let mut kept = Vec::new();
        let mut total_bytes: u64 = 0;
        for cid in referenced_cids(&body.html) {
            if let Some((img_id, file_name, media_type, blob)) = by_cid.remove(&cid) {
                total_bytes = total_bytes.saturating_add(blob.size);
                kept.push((img_id, file_name, media_type, blob, cid));
            }
        }
        if kept.len() > MAX_SIGNATURE_IMAGES {
            return Err(AppError::new(ErrorCode::TooManyAttachments));
        }
        if total_bytes > MAX_SIGNATURE_IMAGE_BYTES {
            return Err(AppError::new(ErrorCode::AttachmentTooLarge));
        }
        let rows: Vec<NewSignatureImage<'_>> = kept
            .iter()
            .map(
                |(img_id, file_name, media_type, blob, cid)| NewSignatureImage {
                    id: img_id,
                    file_name,
                    media_type,
                    blob,
                    content_id: cid,
                },
            )
            .collect();

        let now = now();
        self.storage.save_signature(&SaveSignatureRow {
            id: &id,
            is_new,
            name: &name,
            body_html: &body.html,
            body_text: &body.text,
            images: &rows,
            now: &now,
        })?;
        self.storage.signatures_state()
    }

    pub fn delete_signature(&mut self, id: &str) -> Result<SignaturesState, AppError> {
        validate_uuid(id, "signatureId")?;
        self.storage.delete_signature(id)?;
        self.storage.signatures_state()
    }

    pub fn set_default_signature(
        &mut self,
        request: SetDefaultSignatureRequest,
    ) -> Result<SignaturesState, AppError> {
        if let Some(sig_id) = request.signature_id.as_deref() {
            validate_uuid(sig_id, "signatureId")?;
        }
        self.storage
            .set_global_default_signature(request.signature_id.as_deref())?;
        self.storage.signatures_state()
    }

    pub fn set_account_default_signature(
        &mut self,
        request: SetAccountSignatureRequest,
    ) -> Result<SignaturesState, AppError> {
        validate_uuid(&request.account_id, "accountId")?;
        if let AccountSignatureMode::Specific { signature_id } = &request.selection {
            validate_uuid(signature_id, "signatureId")?;
        }
        self.storage
            .set_account_default_signature(&request.account_id, &request.selection)?;
        self.storage.signatures_state()
    }

    /// Resolves which signature should be inserted for `account_id` and prepares
    /// the signature block HTML plus any inline image copies for the draft.
    pub(crate) fn prepare_signature_for_draft(
        &self,
        account_id: &str,
        selection: Option<&DraftSignatureSelection>,
    ) -> Result<Option<(String, Vec<PreparedSignatureImage>)>, AppError> {
        let sig_id = match selection.unwrap_or(&DraftSignatureSelection::Default) {
            DraftSignatureSelection::None => return Ok(None),
            DraftSignatureSelection::Default => {
                match self.storage.account_default_signature_id(account_id)? {
                    Some(id) => id,
                    None => return Ok(None),
                }
            }
            DraftSignatureSelection::Specific { signature_id } => {
                validate_uuid(signature_id, "signatureId")?;
                signature_id.clone()
            }
        };
        self.materialize_signature_block(account_id, &sig_id)
            .map(Some)
    }

    fn materialize_signature_block(
        &self,
        account_id: &str,
        signature_id: &str,
    ) -> Result<(String, Vec<PreparedSignatureImage>), AppError> {
        let (signature, images) = self.storage.get_signature(signature_id)?;
        let (sender, _) = self.storage.account_sender(account_id)?;
        let mut mapping = HashMap::new();
        let mut prepared = Vec::new();
        let referenced: HashSet<String> =
            referenced_cids(&signature.body_html).into_iter().collect();
        for row in images {
            if !referenced.contains(&row.summary.content_id) {
                continue;
            }
            let new_cid = new_content_id(&sender)?;
            mapping.insert(row.summary.content_id, new_cid.clone());
            prepared.push(PreparedSignatureImage {
                id: new_id(),
                content_id: new_cid,
                file_name: row.summary.file_name,
                media_type: row.summary.media_type,
                blob: row.blob,
            });
        }
        let rewritten = rewrite_cids(&signature.body_html, &mapping);
        let block = signature_block_html(&signature.id, &rewritten);
        Ok((block, prepared))
    }

    /// Replaces or removes the signature block on an existing draft in one revision step.
    pub fn draft_apply_signature(
        &mut self,
        request: ApplyDraftSignatureRequest,
    ) -> Result<Draft, AppError> {
        validate_uuid(&request.draft_id, "draftId")?;
        if let Some(sig_id) = request.signature_id.as_deref() {
            validate_uuid(sig_id, "signatureId")?;
        }
        let draft = self.storage.get_draft(&request.draft_id)?;
        if self.storage.draft_locked(&request.draft_id)? {
            return Err(AppError::new(ErrorCode::DraftLocked));
        }
        if draft.revision != request.expected_revision {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let (block, prepared_images) = match request.signature_id.as_deref() {
            Some(sig_id) => {
                let (block, images) =
                    self.materialize_signature_block(&draft.account_id, sig_id)?;
                (Some(block), images)
            }
            None => (None, Vec::new()),
        };
        let next_html = apply_signature_to_html(&draft.body_html, block.as_deref(), draft.kind);
        let content = validate::normalize(DraftContent {
            to: draft.to,
            cc: draft.cc,
            bcc: draft.bcc,
            subject: draft.subject,
            body_html: next_html,
            body_text: String::new(),
        })?;
        let policy = self.attachment_policy(&draft.account_id)?;
        let rows: Vec<NewAttachment<'_>> = prepared_images
            .iter()
            .map(|img| NewAttachment {
                id: &img.id,
                content_id: Some(&img.content_id),
                file_name: &img.file_name,
                media_type: &img.media_type,
                blob: &img.blob,
                origin: AttachmentOrigin::User,
            })
            .collect();
        self.storage.apply_draft_signature(
            &request.draft_id,
            request.expected_revision,
            &content,
            &rows,
            policy,
            &now(),
        )
    }
}
