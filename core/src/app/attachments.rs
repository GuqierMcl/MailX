//! Saving attachments. The core hands a host the decoded bytes and a safe file
//! name from the message's local raw copy; where those bytes go (a save dialog
//! on the desktop, an HTTP download on a server) is the host's business.
use super::MailxCore;
use crate::{AppError, ErrorCode, mail::filename::safe_file_name, mail::render};

/// Everything about an attachment a host needs before it asks for the bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentInfo {
    /// A single path component, already safe on every supported platform.
    pub file_name: String,
    /// The sender's claim. Hosts must not let it decide how a file is opened or
    /// rendered: a server sends `application/octet-stream` with `attachment`
    /// disposition and `nosniff` whatever this says.
    pub mime_type: String,
    pub size: u64,
}

pub struct AttachmentFile {
    pub info: AttachmentInfo,
    pub bytes: Vec<u8>,
}

fn missing() -> AppError {
    AppError::new(ErrorCode::NotFound)
}

/// Attachment ids are `<email id>:<part index>`, as the email detail lists them.
fn part_index(email_id: &str, attachment_id: &str) -> Result<usize, AppError> {
    attachment_id
        .strip_prefix(email_id)
        .and_then(|rest| rest.strip_prefix(':'))
        .filter(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|index| index.parse().ok())
        .ok_or_else(missing)
}

fn info(part: &render::RenderedPart) -> AttachmentInfo {
    AttachmentInfo {
        file_name: safe_file_name(part.filename.as_deref(), &part.mime_type),
        mime_type: part.mime_type.clone(),
        size: part.size,
    }
}

impl MailxCore {
    /// Metadata of a listed attachment of a downloaded message. Fails with
    /// `NotFound` for a message that is gone, whose body is not available, or an
    /// id that is not one of its listed attachments.
    pub async fn attachment_info(
        &self,
        email_id: String,
        attachment_id: String,
    ) -> Result<AttachmentInfo, AppError> {
        let index = part_index(&email_id, &attachment_id)?;
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            let identity = service.body_identity(&email_id)?;
            let (_, part) = service
                .body_attachment(&identity, index)?
                .ok_or_else(missing)?;
            Ok(info(&part))
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    /// The decoded content, read from the local raw copy: no network. The body
    /// must still be the one that was read when the bytes are returned, so an
    /// attachment of a message that was deleted or re-synced meanwhile is never
    /// handed out.
    pub async fn attachment_content(
        &self,
        email_id: String,
        attachment_id: String,
    ) -> Result<AttachmentFile, AppError> {
        let index = part_index(&email_id, &attachment_id)?;
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            let (identity, raw, part, blobs) = {
                let service = core.accounts()?;
                let identity = service.body_identity(&email_id)?;
                let (raw, part) = service
                    .body_attachment(&identity, index)?
                    .ok_or_else(missing)?;
                (identity, raw, part, service.body_blobs())
            };
            let message = blobs.read(&raw).map_err(|_| missing())?;
            let unchanged = core
                .accounts()?
                .body_stamp(&identity)?
                .is_some_and(|(current, _)| current == raw);
            if !unchanged {
                return Err(missing());
            }
            let content = render::attachment_content(&message, index).map_err(|_| missing())?;
            Ok(AttachmentFile {
                info: info(&part),
                bytes: content.bytes,
            })
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
    }
}
