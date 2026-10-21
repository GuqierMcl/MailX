//! Sender avatars from Gravatar, an opt-in privacy trade-off: asking for one
//! tells Gravatar the SHA-256 of a sender's address. The request goes through
//! the owning account's route like every other remote image, and a route that
//! cannot attest its destination (a custom proxy) simply gets no avatar.
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::MailxCore;
use crate::{AppError, ErrorCode, mail::render::InlineImage};

const FOUND_TTL: Duration = Duration::from_secs(24 * 60 * 60);
// Failures are not distinguished (no profile, offline, blocked route), so retry soon.
const MISSING_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_ENTRIES: usize = 1024;

/// A list can ask for a hundred senders at once. The shared download limit's
/// 30 s timeout also covers waiting for a slot, so without queueing here the
/// tail of the list would time out and be remembered as "no avatar". Fewer than
/// the shared limit, so message images are never starved.
static LOOKUPS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AvatarImage {
    pub mime_type: String,
    /// Base64 of the re-encoded, validated image.
    pub data: String,
}

#[derive(Default)]
pub(super) struct AvatarCache {
    entries: Mutex<HashMap<String, (Instant, Option<AvatarImage>)>>,
}

impl AvatarCache {
    fn get(&self, key: &str) -> Option<Option<AvatarImage>> {
        let entries = self.entries.lock().ok()?;
        let (stored, image) = entries.get(key)?;
        let ttl = if image.is_some() {
            FOUND_TTL
        } else {
            MISSING_TTL
        };
        (stored.elapsed() < ttl).then(|| image.clone())
    }

    fn put(&self, key: String, image: Option<AvatarImage>) {
        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= MAX_ENTRIES {
                entries.retain(|_, (stored, image)| {
                    stored.elapsed()
                        < if image.is_some() {
                            FOUND_TTL
                        } else {
                            MISSING_TTL
                        }
                });
                if entries.len() >= MAX_ENTRIES {
                    entries.clear();
                }
            }
            entries.insert(key, (Instant::now(), image));
        }
    }
}

/// Lowercase, trimmed address in the only shape worth hashing.
fn normalize(address: &str) -> Option<String> {
    let address = address.trim().to_lowercase();
    let (local, domain) = address.split_once('@')?;
    (address.len() <= 254
        && !local.is_empty()
        && domain.contains('.')
        && !domain.contains('@')
        && !address.chars().any(|c| c.is_control() || c.is_whitespace()))
    .then_some(address)
}

fn gravatar_url(address: &str) -> String {
    let hash = Sha256::digest(address.as_bytes());
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    // d=404: a missing profile is an error, not a generated placeholder.
    format!("https://www.gravatar.com/avatar/{hex}?s=96&d=404")
}

impl MailxCore {
    /// `None` when the address has no avatar or none could be fetched. The
    /// caller must already have checked the user's opt-in setting.
    pub async fn sender_avatar(
        &self,
        account_id: String,
        address: String,
    ) -> Result<Option<AvatarImage>, AppError> {
        let Some(address) = normalize(&address) else {
            return Ok(None);
        };
        if let Some(cached) = self.avatars.get(&address) {
            return Ok(cached);
        }
        let core = self.clone();
        let route = tokio::task::spawn_blocking(move || {
            core.accounts()?
                .image_route(&account_id)
                .map_err(|_| AppError::new(ErrorCode::NotFound))
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
        let _slot = LOOKUPS
            .acquire()
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        // Another request for this address may have finished while this one queued.
        if let Some(cached) = self.avatars.get(&address) {
            return Ok(cached);
        }
        let image = crate::net::images::fetch(route, gravatar_url(&address))
            .await
            .ok()
            .map(|InlineImage { bytes, mime_type }| AvatarImage {
                mime_type,
                data: STANDARD.encode(bytes),
            });
        self.avatars.put(address, image.clone());
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalized_before_hashing_and_junk_is_rejected() {
        assert_eq!(
            normalize("  Lin@Example.COM "),
            Some("lin@example.com".into())
        );
        for bad in [
            "",
            "no-at",
            "@example.com",
            "a@b",
            "a@@b.c",
            "a b@c.d",
            "a\n@b.c",
        ] {
            assert_eq!(normalize(bad), None, "{bad:?}");
        }
        assert_eq!(normalize(&format!("{}@x.io", "a".repeat(260))), None);
    }

    #[test]
    fn gravatar_url_uses_the_sha256_of_the_normalized_address_and_never_a_default_image() {
        // SHA-256("lin@example.com")
        let url = gravatar_url("lin@example.com");
        assert!(url.starts_with("https://www.gravatar.com/avatar/"));
        assert!(url.ends_with("?s=96&d=404"));
        assert_eq!(
            url.len(),
            "https://www.gravatar.com/avatar/".len() + 64 + "?s=96&d=404".len()
        );
        assert!(!url.contains("lin@example.com"));
    }

    #[test]
    fn cache_returns_hits_including_misses_and_bounds_its_size() {
        let cache = AvatarCache::default();
        assert!(cache.get("a@b.c").is_none());
        cache.put("a@b.c".into(), None);
        assert_eq!(cache.get("a@b.c"), Some(None));
        let image = AvatarImage {
            mime_type: "image/png".into(),
            data: "AA==".into(),
        };
        cache.put("d@e.f".into(), Some(image.clone()));
        assert_eq!(cache.get("d@e.f"), Some(Some(image)));
        for index in 0..(MAX_ENTRIES + 5) {
            cache.put(format!("u{index}@x.io"), None);
        }
        assert!(cache.entries.lock().unwrap().len() <= MAX_ENTRIES);
    }
}
