//! Host-only body resources. None of these contents or identities is an IPC DTO.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::MailxCore;
use crate::net::images::ImageFetchError;
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
    remote_cancelled: tokio::sync::watch::Sender<bool>,
}

const MAX_REMOTE_CACHE_BYTES: usize = 64 * 1024 * 1024;
const IMAGE_RETRY_COOLDOWN: Duration = Duration::from_secs(2);
/// How long a downloaded message image is reused without asking the sender again.
const REMOTE_IMAGE_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Default)]
struct RemoteImages {
    images: HashMap<String, InlineImage>,
    failures: HashMap<String, ImageFetchError>,
    requests: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    statuses: HashMap<usize, RemoteImageStatus>,
    last_retry: Option<Instant>,
    released: bool,
}

/// Safe diagnostics for the trusted reader. No source URL or response text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct RemoteImageStatus {
    pub index: usize,
    pub loading: bool,
    pub error: Option<ImageFetchError>,
}

fn remote_cache_key(binding: &BodyBinding, url: &str) -> String {
    // Account revision includes route and proxy policy changes. Never reuse the
    // previous global URL cache after introducing delegated proxy DNS.
    format!(
        "remote-v2:{}:{}:{}:{url}",
        binding.identity.account_id,
        binding.identity.mailbox_source_id,
        binding.identity.account_revision,
    )
}

fn unavailable<T>(_: T) -> ImageFetchError {
    ImageFetchError::Cancelled
}

impl BodyBinding {
    pub fn account_id(&self) -> &str {
        &self.identity.account_id
    }

    pub fn email_id(&self) -> &str {
        &self.identity.message_id
    }

    /// Ends image reuse for this reading, including all clones and pending loads.
    pub fn release_remote_images(&self) {
        // Store cancellation even if nobody has subscribed yet. Pending
        // request/dial slots must not turn into new network work after release.
        self.remote_cancelled.send_replace(true);
        if let Ok(mut cache) = self.remote_images.lock() {
            cache.images.clear();
            cache.failures.clear();
            cache.requests.clear();
            cache.statuses.clear();
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

    fn cache_remote_image(&self, url: String, image: &InlineImage) -> Result<(), ImageFetchError> {
        let mut cache = self.remote_images.lock().map_err(unavailable)?;
        if cache.released {
            return Err(ImageFetchError::Cancelled);
        }
        let retained: usize = cache.images.values().map(|image| image.bytes.len()).sum();
        if !cache.images.contains_key(&url) {
            // Every successful response remains available for a retry even if
            // the optional disk cache cannot retain it. Budget exhaustion is a
            // visible failure, never a silently uncached successful response.
            if image.bytes.len() > MAX_REMOTE_CACHE_BYTES.saturating_sub(retained) {
                cache.failures.insert(url, ImageFetchError::TooLarge);
                return Err(ImageFetchError::TooLarge);
            }
            cache.images.insert(url, image.clone());
        }
        Ok(())
    }

    fn image_status(&self, status: RemoteImageStatus) -> Result<(), AppError> {
        let mut cache = self.remote_images.lock().map_err(|_| internal())?;
        if cache.released {
            return Err(missing());
        }
        if status.loading || cache.statuses.contains_key(&status.index) {
            cache.statuses.insert(status.index, status);
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
            remote_cancelled: tokio::sync::watch::channel(false).0,
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
                    remote_cancelled: tokio::sync::watch::channel(false).0,
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
    /// Validated bytes are cached in this binding and in an account/revision
    /// scoped disk cache. Cache hits never authorize an otherwise invalid read.
    pub async fn body_remote_image(
        &self,
        binding: BodyBinding,
        index: usize,
    ) -> Result<InlineImage, AppError> {
        self.body_remote_image_detailed(binding, index)
            .await
            .map_err(Into::into)
    }

    pub async fn body_remote_image_detailed(
        &self,
        binding: BodyBinding,
        index: usize,
    ) -> Result<InlineImage, ImageFetchError> {
        // A caller that drops this future mid-flight would otherwise leave the
        // image `loading` forever and block every later retry.
        struct DropGuard<'a> {
            binding: &'a BodyBinding,
            index: usize,
            finished: bool,
        }
        impl Drop for DropGuard<'_> {
            fn drop(&mut self) {
                if !self.finished {
                    let _ = self.binding.image_status(RemoteImageStatus {
                        index: self.index,
                        loading: false,
                        error: Some(ImageFetchError::Cancelled),
                    });
                }
            }
        }
        let mut guard = DropGuard {
            binding: &binding,
            index,
            finished: false,
        };
        let mut cancelled = binding.remote_cancelled.subscribe();
        let result = if *cancelled.borrow() {
            Err(ImageFetchError::Cancelled)
        } else {
            tokio::select! {
                biased;
                _ = cancelled.changed() => Err(ImageFetchError::Cancelled),
                result = self.load_remote_image(&binding, index) => result,
            }
        };
        // Released bindings cannot regain status through a late completion.
        let _ = binding.image_status(RemoteImageStatus {
            index,
            loading: false,
            error: result.as_ref().err().copied(),
        });
        guard.finished = true;
        result
    }

    /// A read-only snapshot for a trusted host, not for the mail iframe.
    pub async fn body_image_status(
        &self,
        binding: BodyBinding,
    ) -> Result<Vec<RemoteImageStatus>, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            check(&*core.accounts()?, &binding)?;
            let cache = binding.remote_images.lock().map_err(|_| internal())?;
            if cache.released {
                return Err(missing());
            }
            let mut statuses: Vec<_> = cache.statuses.values().cloned().collect();
            statuses.sort_by_key(|status| status.index);
            Ok(statuses)
        })
        .await
        .map_err(|_| internal())?
    }

    /// A trusted host can issue a fresh document token for this independent
    /// reading. Only failed resources become eligible for a new fetch; already
    /// successful images survive without relying on the optional disk cache.
    pub async fn body_retry_images(&self, binding: BodyBinding) -> Result<BodyBinding, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            check(&*core.accounts()?, &binding)?;
            let mut cache = binding.remote_images.lock().map_err(|_| internal())?;
            if cache.released || *binding.remote_cancelled.borrow() {
                return Err(missing());
            }
            if cache.statuses.values().any(|status| status.loading)
                || cache
                    .last_retry
                    .is_some_and(|last| last.elapsed() < IMAGE_RETRY_COOLDOWN)
            {
                return Err(AppError::validation("imageRetry"));
            }
            let now = Instant::now();
            cache.last_retry = Some(now);
            let images = RemoteImages {
                images: cache.images.clone(),
                statuses: cache
                    .statuses
                    .iter()
                    .filter(|(_, status)| status.error.is_none())
                    .map(|(&index, status)| (index, status.clone()))
                    .collect(),
                last_retry: Some(now),
                ..Default::default()
            };
            Ok(BodyBinding {
                identity: binding.identity.clone(),
                raw: binding.raw.clone(),
                sanitizer_version: binding.sanitizer_version,
                remote_images: Arc::new(Mutex::new(images)),
                remote_cancelled: tokio::sync::watch::channel(false).0,
            })
        })
        .await
        .map_err(|_| internal())?
    }

    async fn load_remote_image(
        &self,
        binding: &BodyBinding,
        index: usize,
    ) -> Result<InlineImage, ImageFetchError> {
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
        .map_err(unavailable)?
        .map_err(unavailable)?;
        binding
            .image_status(RemoteImageStatus {
                index,
                loading: true,
                error: None,
            })
            .map_err(unavailable)?;
        let request = {
            let mut cache = binding.remote_images.lock().map_err(unavailable)?;
            if cache.released {
                return Err(ImageFetchError::Cancelled);
            }
            cache.requests.entry(url.clone()).or_default().clone()
        };
        let _request = request.lock().await;
        if let Some(image) = binding.cached_remote_image(&url).map_err(unavailable)? {
            check(&*self.accounts().map_err(unavailable)?, binding).map_err(unavailable)?;
            return Ok(image);
        }
        if let Some(error) = binding
            .remote_images
            .lock()
            .map_err(unavailable)?
            .failures
            .get(&url)
        {
            return Err(*error);
        }
        let cache_key = remote_cache_key(binding, &url);
        if let Some(image) = self.images.get(&cache_key, REMOTE_IMAGE_TTL) {
            check(&*self.accounts().map_err(unavailable)?, binding).map_err(unavailable)?;
            binding.cache_remote_image(url, &image)?;
            return Ok(image);
        }
        let core = self.clone();
        let saved = binding.clone();
        let route = tokio::task::spawn_blocking(move || {
            let service = core.accounts().map_err(unavailable)?;
            check(&service, &saved).map_err(unavailable)?;
            service
                .image_route(&saved.identity.account_id)
                .map_err(|error| match error {
                    crate::accounts::sync_plan::PlanError::CredentialMissing
                    | crate::accounts::sync_plan::PlanError::CredentialUntrusted => {
                        ImageFetchError::Proxy
                    }
                    crate::accounts::sync_plan::PlanError::Account(_) => ImageFetchError::Cancelled,
                })
        })
        .await
        .map_err(unavailable)??;
        let image = match crate::net::images::fetch(&route, url.clone()).await {
            Ok(image) => image,
            Err(error) => {
                let mut cache = binding.remote_images.lock().map_err(unavailable)?;
                if !cache.released {
                    cache.failures.insert(url, error);
                }
                return Err(error);
            }
        };
        check(&*self.accounts().map_err(unavailable)?, binding).map_err(unavailable)?;
        binding.cache_remote_image(url, &image)?;
        self.images.put(&cache_key, &image);
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
