//! Host-only body resources. None of these contents or identities is an IPC DTO.
use super::MailxCore;
use crate::{
    AppError, ErrorCode,
    accounts::AccountService,
    mail::render::{self, InlineImage, SANITIZER_VERSION},
    storage::bodies::{BodyIdentity, StoredBlob},
};

/// A host can retain this opaque value alongside a random document token. It
/// cannot construct a binding from an untrusted URL or frontend request.
#[derive(Clone)]
pub struct BodyBinding {
    identity: BodyIdentity,
    raw: StoredBlob,
    sanitizer_version: u32,
}

impl BodyBinding {
    pub fn email_id(&self) -> &str {
        &self.identity.message_id
    }
}

/// HTML is deliberately neither Debug nor Serialize. Serve only through the
/// isolated protocol, after resolving renderer-created DOM resource bindings.
pub struct BodyDocument {
    pub html: String,
    pub binding: BodyBinding,
}

fn missing() -> AppError {
    AppError::new(ErrorCode::NotFound)
}
fn internal() -> AppError {
    AppError::new(ErrorCode::Internal)
}

fn check(service: &AccountService, binding: &BodyBinding) -> Result<(), AppError> {
    if binding.sanitizer_version != SANITIZER_VERSION
        || service.body_stamp(&binding.identity)?.as_ref()
            != Some(&(binding.raw.clone(), binding.sanitizer_version))
    {
        return Err(missing());
    }
    Ok(())
}

impl MailxCore {
    /// A URL read never authorizes a new download. Only already-cached bodies
    /// can be served; an older sanitizer version is rebuilt from the local blob.
    pub async fn body_document(&self, email_id: String) -> Result<BodyDocument, AppError> {
        self.bodies.refresh(email_id.clone()).await?;
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            let identity = service.body_identity(&email_id)?;
            let body = service.body_cache(&identity)?.ok_or_else(missing)?;
            if body.sanitizer_version != SANITIZER_VERSION {
                return Err(missing());
            }
            Ok(BodyDocument {
                html: body.rendered.sanitized_html.ok_or_else(missing)?,
                binding: BodyBinding {
                    identity,
                    raw: body.raw,
                    sanitizer_version: body.sanitizer_version,
                },
            })
        })
        .await
        .map_err(|_| internal())?
    }

    pub async fn body_part(
        &self,
        binding: BodyBinding,
        content_id: String,
    ) -> Result<InlineImage, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            let (blobs, raw, part) = {
                let service = core.accounts()?;
                check(&service, &binding)?;
                let (raw, part) = service
                    .body_part(&binding.identity, &content_id)?
                    .ok_or_else(missing)?;
                (service.body_blobs(), raw, part)
            };
            // Full decoding and filesystem I/O never hold the service mutex.
            let raw = blobs.read(&raw)?;
            let image =
                render::inline_image(&raw, part.part_index, &content_id).map_err(|_| missing())?;
            check(&*core.accounts()?, &binding)?;
            Ok(image)
        })
        .await
        .map_err(|_| internal())?
    }

    /// Protocol requests supply their opaque binding. The trusted confirmation
    /// command can omit it, but still re-reads identity, index and allowed scheme.
    pub async fn body_link(
        &self,
        email_id: String,
        index: usize,
        binding: Option<BodyBinding>,
    ) -> Result<String, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            let identity = service.body_identity(&email_id)?;
            if let Some(binding) = binding {
                if binding.email_id() != email_id {
                    return Err(missing());
                }
                check(&service, &binding)?;
            }
            let stamp = service.body_stamp(&identity)?.ok_or_else(missing)?;
            if stamp.1 != SANITIZER_VERSION {
                return Err(missing());
            }
            service
                .body_link(&identity, index)?
                .filter(|url| render::is_allowed_link(url))
                .ok_or_else(missing)
        })
        .await
        .map_err(|_| internal())?
    }
}

#[cfg(test)]
mod tests;
