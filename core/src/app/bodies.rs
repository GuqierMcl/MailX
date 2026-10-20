//! Host-only body resources. None of these contents or identities is an IPC DTO.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

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
    remote_images: Arc<Mutex<RemoteImages>>,
}

const MAX_REMOTE_CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
struct RemoteImages {
    images: HashMap<String, InlineImage>,
    released: bool,
}

impl BodyBinding {
    pub fn email_id(&self) -> &str {
        &self.identity.message_id
    }

    /// Ends image reuse for this reading, including all clones and pending loads.
    pub fn release_remote_images(&self) {
        if let Ok(mut cache) = self.remote_images.lock() {
            cache.images.clear();
            cache.released = true;
        }
    }

    fn cached_remote_image(&self, url: &str) -> Result<Option<InlineImage>, AppError> {
        let cache = self.remote_images.lock().map_err(|_| internal())?;
        if cache.released {
            return Err(missing());
        }
        Ok(cache.images.get(url).cloned())
    }

    fn cache_remote_image(&self, url: String, image: &InlineImage) -> Result<(), AppError> {
        let mut cache = self.remote_images.lock().map_err(|_| internal())?;
        if cache.released {
            return Err(missing());
        }
        let retained: usize = cache.images.values().map(|image| image.bytes.len()).sum();
        if !cache.images.contains_key(&url)
            && retained + image.bytes.len() <= MAX_REMOTE_CACHE_BYTES
        {
            cache.images.insert(url, image.clone());
        }
        Ok(())
    }
}

/// HTML is deliberately neither Debug nor Serialize. Serve only through the
/// isolated protocol, after resolving renderer-created DOM resource bindings.
pub struct BodyDocument {
    pub html: String,
    pub binding: BodyBinding,
}

/// Host-only numbered links retain the exact database order and an opaque
/// binding. This is not a serializable application DTO or an opener grant.
pub struct BodyLinks {
    pub links: Vec<String>,
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

/// `None` requests one local sanitizer refresh; missing/invisible bodies fail.
fn read_links(
    service: &AccountService,
    email_id: &str,
    binding: Option<&BodyBinding>,
) -> Result<Option<BodyLinks>, AppError> {
    let identity = service.body_identity(email_id)?;
    if let Some(binding) = binding {
        if binding.email_id() != email_id {
            return Err(missing());
        }
        check(service, binding)?;
    }
    let (raw, sanitizer_version) = service.body_stamp(&identity)?.ok_or_else(missing)?;
    if sanitizer_version != SANITIZER_VERSION {
        return Ok(None);
    }
    let links = service.body_links(&identity)?.ok_or_else(missing)?;
    // Dropping an invalid item would renumber every subsequent link. Reject the
    // whole list instead; confirmation must keep the original database indices.
    if links.iter().any(|url| !render::is_allowed_link(url)) {
        return Err(missing());
    }
    Ok(Some(BodyLinks {
        links,
        binding: BodyBinding {
            identity,
            raw,
            sanitizer_version,
            remote_images: Arc::default(),
        },
    }))
}

impl MailxCore {
    /// Enumerate already-cached links for a trusted host without loading HTML.
    /// An unbound outdated cache may rebuild locally; a supplied document binding
    /// must still be current and is never silently rebound. Missing bodies never download.
    pub async fn body_links(
        &self,
        email_id: String,
        binding: Option<BodyBinding>,
    ) -> Result<BodyLinks, AppError> {
        let read = || {
            let core = self.clone();
            let email_id = email_id.clone();
            let binding = binding.clone();
            tokio::task::spawn_blocking(move || {
                read_links(&*core.accounts()?, &email_id, binding.as_ref())
            })
        };
        if let Some(links) = read().await.map_err(|_| internal())?? {
            return Ok(links);
        }
        self.bodies.refresh(email_id.clone()).await?;
        read().await.map_err(|_| internal())??.ok_or_else(missing)
    }

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
                    remote_images: Arc::default(),
                },
            })
        })
        .await
        .map_err(|_| internal())?
    }

    /// Serve a previously prepared document without refreshing or rebinding it.
    /// Identity validation and the HTML read share the account-service lock.
    pub async fn body_document_bound(
        &self,
        binding: BodyBinding,
    ) -> Result<BodyDocument, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            check(&service, &binding)?;
            let body = service.body_cache(&binding.identity)?.ok_or_else(missing)?;
            if body.raw != binding.raw || body.sanitizer_version != binding.sanitizer_version {
                return Err(missing());
            }
            Ok(BodyDocument {
                html: body.rendered.sanitized_html.ok_or_else(missing)?,
                binding,
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

    /// Loads only a renderer-created image index from a current document. The
    /// trusted host must check its image policy/document grant before calling.
    /// Validated bytes live only in this reading binding; nothing is persisted.
    pub async fn body_remote_image(
        &self,
        binding: BodyBinding,
        index: usize,
    ) -> Result<InlineImage, AppError> {
        let core = self.clone();
        let saved = binding.clone();
        let url = tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            check(&service, &saved)?;
            let body = service.body_cache(&saved.identity)?.ok_or_else(missing)?;
            render::remote_images(
                body.rendered
                    .sanitized_html
                    .as_deref()
                    .ok_or_else(missing)?,
            )
            .get(index)
            .cloned()
            .ok_or_else(missing)
        })
        .await
        .map_err(|_| internal())??;
        if let Some(image) = binding.cached_remote_image(&url)? {
            check(&*self.accounts()?, &binding)?;
            return Ok(image);
        }
        let core = self.clone();
        let saved = binding.clone();
        let route = tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            check(&service, &saved)?;
            service
                .image_route(&saved.identity.account_id)
                .map_err(|_| missing())
        })
        .await
        .map_err(|_| internal())??;
        let image = crate::net::images::fetch(route, url.clone()).await?;
        check(&*self.accounts()?, &binding)?;
        binding.cache_remote_image(url, &image)?;
        Ok(image)
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
