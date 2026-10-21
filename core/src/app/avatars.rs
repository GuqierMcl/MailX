//! Sender avatars from public sources, an opt-in privacy trade-off.
//!
//! Order for one sender: Gravatar by the SHA-256 of the address, then, unless
//! the address belongs to a personal mailbox provider, the sender's domain icon
//! from DuckDuckGo and then Google. Personal providers are skipped on purpose:
//! every Gmail sender would otherwise get Google's logo. All requests use the
//! owning account's route like any remote image, and a route that cannot attest
//! its destination (a custom proxy) gets no avatar rather than a direct request.
//!
//! Disclosure: Gravatar learns a hash of the address; the icon services learn
//! the sender's domain in clear text. Found images persist in the shared image
//! cache; misses are only remembered in memory, for a shorter time.
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
use crate::{AppError, ErrorCode, mail::render::InlineImage, net::NetRoute};

const FOUND_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
// Failures are not distinguished (no profile, offline, blocked route), so retry soon.
const MISSING_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_ENTRIES: usize = 1024;

/// A list can ask for a hundred senders at once. The shared download limit's
/// 30 s timeout also covers waiting for a slot, so without queueing here the
/// tail of the list would time out and be remembered as "no avatar". Fewer than
/// the shared limit, so message images are never starved.
static LOOKUPS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AvatarKind {
    /// A person's picture, shown cropped to the circle.
    Photo,
    /// A small site icon, shown whole on a plain background.
    Icon,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AvatarImage {
    pub mime_type: String,
    /// Base64 of the re-encoded, validated image.
    pub data: String,
    pub kind: AvatarKind,
}

impl AvatarImage {
    fn new(image: InlineImage, kind: AvatarKind) -> Self {
        Self {
            mime_type: image.mime_type,
            data: STANDARD.encode(image.bytes),
            kind,
        }
    }
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

    pub(super) fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
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

/// Mailbox providers whose senders are people, not organizations. Matching
/// includes subdomains (`mail.qq.com`). Domains of the user's own accounts are
/// added at lookup time, so a provider missing here still behaves once the user
/// has an account there.
const PERSONAL_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "passport.com",
    "yahoo.com",
    "yahoo.co.jp",
    "ymail.com",
    "rocketmail.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "proton.me",
    "protonmail.com",
    "pm.me",
    "tutanota.com",
    "tuta.io",
    "fastmail.com",
    "zoho.com",
    "gmx.com",
    "gmx.net",
    "mail.com",
    "yandex.com",
    "yandex.ru",
    "mail.ru",
    "qq.com",
    "vip.qq.com",
    "foxmail.com",
    "163.com",
    "126.com",
    "yeah.net",
    "188.com",
    "vip.163.com",
    "vip.126.com",
    "sina.com",
    "sina.cn",
    "vip.sina.com",
    "sohu.com",
    "vip.sohu.com",
    "139.com",
    "189.cn",
    "wo.cn",
    "aliyun.com",
    "tom.com",
    "21cn.com",
    "2925.com",
    "eyou.com",
    "china.com",
    "88.com",
    "zoho.com.cn",
    "web.de",
    "naver.com",
    "daum.net",
    "hanmail.net",
    "t-online.de",
    "orange.fr",
    "free.fr",
    "laposte.net",
    "libero.it",
    "btinternet.com",
    "comcast.net",
    "verizon.net",
    "att.net",
    "sbcglobal.net",
];

fn in_domain(domain: &str, listed: &str) -> bool {
    domain == listed
        || domain
            .strip_suffix(listed)
            .is_some_and(|rest| rest.ends_with('.'))
}

fn is_personal(domain: &str, own: &[String]) -> bool {
    PERSONAL_DOMAINS
        .iter()
        .any(|listed| in_domain(domain, listed))
        || own.iter().any(|mine| in_domain(domain, mine))
}

/// Second-level labels that make `example.com.cn` rather than `com.cn` the
/// registrable name. A heuristic in place of the public suffix list: an unlisted
/// multi-part suffix only means one extra lookup for the wrong parent.
const COMPOUND_SECOND_LEVELS: &[&str] =
    &["com", "net", "org", "edu", "gov", "ac", "co", "ne", "or"];

fn icon_domains(domain: &str) -> Vec<String> {
    let labels: Vec<&str> = domain.split('.').collect();
    let valid = labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    if !valid || labels.len() < 2 || domain.len() > 253 {
        return Vec::new();
    }
    let compound = labels.len() >= 3
        && labels[labels.len() - 1].len() == 2
        && COMPOUND_SECOND_LEVELS.contains(&labels[labels.len() - 2]);
    let keep = if compound { 3 } else { 2 };
    let registrable = labels[labels.len().saturating_sub(keep)..].join(".");
    let mut domains = vec![domain.to_owned()];
    if registrable != domain {
        domains.push(registrable);
    }
    domains
}

fn icon_urls(domain: &str) -> [String; 2] {
    [
        format!("https://icons.duckduckgo.com/ip3/{domain}.ico"),
        format!("https://www.google.com/s2/favicons?domain={domain}&sz=128"),
    ]
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
        let memory_key = format!("avatar:{address}");
        if let Some(cached) = self.avatars.get(&memory_key) {
            return Ok(cached);
        }
        if let Some(image) = self.images.get(&memory_key, FOUND_TTL) {
            let image = AvatarImage::new(image, AvatarKind::Photo);
            self.avatars.put(memory_key, Some(image.clone()));
            return Ok(Some(image));
        }
        let domain = address
            .rsplit_once('@')
            .map_or("", |(_, domain)| domain)
            .to_owned();
        let core = self.clone();
        let (route, personal) = tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            let own: Vec<String> = service
                .list()?
                .iter()
                .filter_map(|account| {
                    account
                        .email_address
                        .rsplit_once('@')
                        .map(|(_, domain)| domain.trim().to_lowercase())
                })
                .collect();
            let personal = is_personal(&domain, &own);
            let route = service
                .image_route(&account_id)
                .map_err(|_| AppError::new(ErrorCode::NotFound))?;
            Ok::<_, AppError>((route, personal))
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
        let _slot = LOOKUPS
            .acquire()
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        // Another request for this address may have finished while this one queued.
        if let Some(cached) = self.avatars.get(&memory_key) {
            return Ok(cached);
        }
        let mut found = crate::net::images::fetch(&route, gravatar_url(&address))
            .await
            .ok()
            .map(|image| AvatarImage::new(image, AvatarKind::Photo));
        if found.is_none() && !personal {
            let domain = address.rsplit_once('@').map_or("", |(_, domain)| domain);
            found = self.domain_icon(&route, domain).await;
        }
        // Only photos are stored under the address. A site icon is stored under
        // its domain by `domain_icon`, so a restart reads it back as an icon.
        if let Some(image) = found
            .as_ref()
            .filter(|image| image.kind == AvatarKind::Photo)
            && let Ok(bytes) = STANDARD.decode(&image.data)
        {
            self.images.put(
                &memory_key,
                &InlineImage {
                    bytes,
                    mime_type: image.mime_type.clone(),
                },
            );
        }
        self.avatars.put(memory_key, found.clone());
        Ok(found)
    }

    /// Site icon for a sender's domain, shared by every sender at that domain.
    async fn domain_icon(&self, route: &NetRoute, domain: &str) -> Option<AvatarImage> {
        for candidate in icon_domains(domain) {
            let key = format!("icon:{candidate}");
            if let Some(cached) = self.avatars.get(&key) {
                if cached.is_some() {
                    return cached;
                }
                continue;
            }
            if let Some(image) = self.images.get(&key, FOUND_TTL) {
                let image = AvatarImage::new(image, AvatarKind::Icon);
                self.avatars.put(key, Some(image.clone()));
                return Some(image);
            }
            for url in icon_urls(&candidate) {
                if let Ok(image) = crate::net::images::fetch(route, url).await {
                    self.images.put(&key, &image);
                    let image = AvatarImage::new(image, AvatarKind::Icon);
                    self.avatars.put(key, Some(image.clone()));
                    return Some(image);
                }
            }
            self.avatars.put(key, None);
        }
        None
    }

    /// Deletes cached avatars and mail images, on disk and in memory.
    pub fn clear_image_cache(&self) -> u64 {
        self.avatars.clear();
        self.images.clear()
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
    fn personal_providers_and_the_users_own_domains_skip_site_icons() {
        let own = vec!["corp.example".to_owned()];
        for personal in [
            "gmail.com",
            "mail.qq.com",
            "vip.163.com",
            "2925.com",
            "outlook.com",
        ] {
            assert!(is_personal(personal, &own), "{personal}");
        }
        assert!(is_personal("corp.example", &own));
        assert!(is_personal("eu.corp.example", &own));
        for organization in [
            "github.com",
            "notion.so",
            "notgmail.com",
            "gmail.com.evil.test",
            "mail.notion.so",
        ] {
            assert!(!is_personal(organization, &own), "{organization}");
        }
    }

    #[test]
    fn icon_domains_try_the_exact_domain_then_a_registrable_parent() {
        assert_eq!(icon_domains("github.com"), ["github.com"]);
        assert_eq!(
            icon_domains("mail.notion.so"),
            ["mail.notion.so", "notion.so"]
        );
        assert_eq!(
            icon_domains("em.news.example.co.uk"),
            ["em.news.example.co.uk", "example.co.uk"]
        );
        assert_eq!(
            icon_domains("shop.example.com.cn"),
            ["shop.example.com.cn", "example.com.cn"]
        );
        assert_eq!(icon_domains("example.com.cn"), ["example.com.cn"]);
    }

    #[test]
    fn only_plain_ascii_domains_ever_reach_an_icon_url() {
        for bad in [
            "",
            "localhost",
            "a..b.com",
            "-a.com",
            "a-.com",
            "例え.jp",
            "a.com/x",
            "a.com?x=1",
            "a b.com",
            "a.com:80",
            "under_score.com",
            "A.com",
        ] {
            assert!(icon_domains(bad).is_empty(), "{bad:?}");
        }
        let long = format!("{}.com", "a".repeat(64));
        assert!(icon_domains(&long).is_empty());
        let urls = icon_urls("notion.so");
        assert_eq!(urls[0], "https://icons.duckduckgo.com/ip3/notion.so.ico");
        assert_eq!(
            urls[1],
            "https://www.google.com/s2/favicons?domain=notion.so&sz=128"
        );
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
            kind: AvatarKind::Photo,
        };
        cache.put("d@e.f".into(), Some(image.clone()));
        assert_eq!(cache.get("d@e.f"), Some(Some(image)));
        for index in 0..(MAX_ENTRIES + 5) {
            cache.put(format!("u{index}@x.io"), None);
        }
        assert!(cache.entries.lock().unwrap().len() <= MAX_ENTRIES);
        cache.clear();
        assert!(cache.get("d@e.f").is_none());
    }
}
