//! Sender avatars from public sources, an opt-in privacy trade-off.
//!
//! Order for one sender: Gravatar by the SHA-256 of the address, then, unless
//! the address belongs to a personal mailbox provider, the sender's domain icon
//! from DuckDuckGo and then Google. Personal providers are skipped on purpose:
//! every Gmail sender would otherwise get Google's logo. All requests use the
//! owning account's route and explicit image proxy policy. A failed proxy never
//! falls back to a direct request. Cache entries are isolated by account/source
//! and connection revision.
//!
//! Disclosure: Gravatar learns a hash of the address; the icon services learn
//! the sender's domain in clear text. Found images persist in the shared image
//! cache. Failed lookups are not cached, so a temporary network error does not
//! hide avatars for an hour.
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Mutex,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::MailxCore;
use crate::{
    AppError, ErrorCode,
    accounts::AccountService,
    mail::render::InlineImage,
    net::{NetRoute, images::ImageFetchError},
};

const FOUND_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
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
    entries: Mutex<HashMap<String, (Instant, AvatarImage)>>,
}

impl AvatarCache {
    fn get(&self, key: &str) -> Option<AvatarImage> {
        let entries = self.entries.lock().ok()?;
        let (stored, image) = entries.get(key)?;
        (stored.elapsed() < FOUND_TTL).then(|| image.clone())
    }

    fn put(&self, key: String, image: AvatarImage) {
        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= MAX_ENTRIES {
                entries.retain(|_, (stored, _)| stored.elapsed() < FOUND_TTL);
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

#[derive(Clone)]
struct AvatarBinding {
    account_id: String,
    mailbox_source_id: String,
    revision: u32,
}

impl AvatarBinding {
    fn read(service: &AccountService, account_id: String) -> Result<Self, AppError> {
        let identity = service.sync_identity(&account_id)?;
        Ok(Self {
            account_id,
            mailbox_source_id: identity.mailbox_source_id,
            revision: identity.revision,
        })
    }

    fn check(&self, service: &AccountService) -> Result<(), AppError> {
        let current = service.sync_identity(&self.account_id)?;
        if current.mailbox_source_id != self.mailbox_source_id || current.revision != self.revision
        {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        Ok(())
    }

    fn key(&self, kind: &str, value: &str) -> String {
        format!(
            "avatar-v2:{}:{}:{}:{kind}:{value}",
            self.account_id, self.mailbox_source_id, self.revision
        )
    }
}

type ImageFuture<'a> =
    Pin<Box<dyn Future<Output = Result<InlineImage, ImageFetchError>> + Send + 'a>>;

impl MailxCore {
    /// The caller checks the independent avatar opt-in before and after this
    /// request. No remote-image display grant is implied by an avatar lookup.
    pub async fn sender_avatar(
        &self,
        account_id: String,
        address: String,
    ) -> Result<Option<AvatarImage>, AppError> {
        self.sender_avatar_with(account_id, address, |route, url| {
            Box::pin(crate::net::images::fetch(route, url))
        })
        .await
    }

    async fn sender_avatar_with<F>(
        &self,
        account_id: String,
        address: String,
        fetch: F,
    ) -> Result<Option<AvatarImage>, AppError>
    where
        F: for<'a> Fn(&'a NetRoute, String) -> ImageFuture<'a> + Send + Sync,
    {
        let Some(address) = normalize(&address) else {
            return Ok(None);
        };
        let core = self.clone();
        let binding = tokio::task::spawn_blocking(move || {
            AvatarBinding::read(&*core.accounts()?, account_id)
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
        let memory_key = binding.key("photo", &address);
        if let Some(cached) = self
            .cached_avatar(&binding, &memory_key, AvatarKind::Photo)
            .await?
        {
            return Ok(Some(cached));
        }
        let domain = address
            .rsplit_once('@')
            .map_or("", |(_, domain)| domain)
            .to_owned();
        let core = self.clone();
        let saved = binding.clone();
        let (route, personal) = tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            saved.check(&service)?;
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
            let route = service
                .image_route(&saved.account_id)
                .map_err(|_| AppError::new(ErrorCode::NotFound))?;
            Ok::<_, AppError>((route, is_personal(&domain, &own)))
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
        let _slot = LOOKUPS
            .acquire()
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        // This also rechecks identity after queueing, before a new request.
        if let Some(cached) = self
            .cached_avatar(&binding, &memory_key, AvatarKind::Photo)
            .await?
        {
            return Ok(Some(cached));
        }
        let fetched = fetch(&route, gravatar_url(&address)).await;
        self.check_avatar(&binding).await?;
        if let Ok(image) = fetched {
            return self
                .store_avatar(&binding, &memory_key, image, AvatarKind::Photo)
                .await
                .map(Some);
        }
        let found = if personal {
            None
        } else {
            let domain = address.rsplit_once('@').map_or("", |(_, domain)| domain);
            self.domain_icon(&binding, &route, domain, &fetch).await?
        };
        let core = self.clone();
        let saved = binding.clone();
        tokio::task::spawn_blocking(move || {
            let service = core.accounts()?;
            saved.check(&service)?;
            if let Some(image) = &found {
                // Domain icons stay on disk under their icon key so their kind
                // is preserved across restarts; only memory also aliases sender.
                core.avatars.put(memory_key, image.clone());
            }
            Ok(found)
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    async fn check_avatar(&self, binding: &AvatarBinding) -> Result<(), AppError> {
        let core = self.clone();
        let binding = binding.clone();
        tokio::task::spawn_blocking(move || binding.check(&*core.accounts()?))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    async fn cached_avatar(
        &self,
        binding: &AvatarBinding,
        key: &str,
        kind: AvatarKind,
    ) -> Result<Option<AvatarImage>, AppError> {
        let core = self.clone();
        let binding = binding.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            binding.check(&*core.accounts()?)?;
            let cached = core.avatars.get(&key).or_else(|| {
                core.images
                    .get(&key, FOUND_TTL)
                    .map(|image| AvatarImage::new(image, kind))
            });
            let service = core.accounts()?;
            binding.check(&service)?;
            if let Some(image) = &cached {
                core.avatars.put(key, image.clone());
            }
            Ok(cached)
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    async fn store_avatar(
        &self,
        binding: &AvatarBinding,
        key: &str,
        image: InlineImage,
        kind: AvatarKind,
    ) -> Result<AvatarImage, AppError> {
        let core = self.clone();
        let binding = binding.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            binding.check(&*core.accounts()?)?;
            core.images.put(&key, &image);
            let service = core.accounts()?;
            binding.check(&service)?;
            let image = AvatarImage::new(image, kind);
            core.avatars.put(key, image.clone());
            Ok(image)
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    /// Shared only among senders in the same account/source/connection revision.
    async fn domain_icon<F>(
        &self,
        binding: &AvatarBinding,
        route: &NetRoute,
        domain: &str,
        fetch: &F,
    ) -> Result<Option<AvatarImage>, AppError>
    where
        F: for<'a> Fn(&'a NetRoute, String) -> ImageFuture<'a> + Send + Sync,
    {
        for candidate in icon_domains(domain) {
            let key = binding.key("icon", &candidate);
            if let Some(cached) = self.cached_avatar(binding, &key, AvatarKind::Icon).await? {
                return Ok(Some(cached));
            }
            for url in icon_urls(&candidate) {
                self.check_avatar(binding).await?;
                let fetched = fetch(route, url).await;
                self.check_avatar(binding).await?;
                if let Ok(image) = fetched {
                    return self
                        .store_avatar(binding, &key, image, AvatarKind::Icon)
                        .await
                        .map(Some);
                }
            }
        }
        // No negative cache: HTTP status alone cannot distinguish a missing
        // picture from an upstream outage, and network errors are recoverable.
        Ok(None)
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
    use crate::accounts::{
        sync_plan::tests::{config, connection_update},
        types::{Account, CreateAccountRequest},
    };
    use crate::secrets::test_support::MemorySecrets;
    use serde_json::{Value, json};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tempfile::TempDir;

    fn fixture() -> (TempDir, MailxCore, Account) {
        let directory = TempDir::new().unwrap();
        let core = MailxCore::open(
            directory.path(),
            Arc::new(MemorySecrets::default()),
            tokio::runtime::Handle::current(),
            Arc::new(|_: &crate::events::CoreEvent| {}),
        )
        .unwrap();
        let request: CreateAccountRequest = serde_json::from_value(json!({
            "requestId":uuid::Uuid::new_v4().to_string(), "config":config(json!({"kind":"direct"}))
        }))
        .unwrap();
        // Add after opening, directly through the service: no real sync starts.
        let account = core.accounts().unwrap().create(request).unwrap();
        (directory, core, account)
    }

    fn update_route(core: &MailxCore, account: &Account, route: Value) -> Account {
        let mut input = config(route);
        input["authentication"]["password"] = json!({"action":"keep"});
        core.accounts()
            .unwrap()
            .update_connection(connection_update(&account.id, account.revision, input))
            .unwrap()
    }

    fn synthetic_image(value: u8) -> InlineImage {
        let mut bytes = std::io::Cursor::new(Vec::new());
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([value, 0, 0]),
        ));
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        InlineImage {
            bytes: bytes.into_inner(),
            mime_type: "image/png".into(),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn caches_are_scoped_to_current_account_revision_and_ignore_legacy_keys() {
        let (_directory, core, account) = fixture();
        let address = "sender@gmail.com";
        let old = synthetic_image(10);
        core.images.put(&format!("avatar:{address}"), &old);
        core.avatars.put(
            format!("avatar:{address}"),
            AvatarImage::new(old.clone(), AvatarKind::Photo),
        );
        let calls = AtomicUsize::new(0);
        let loaded = core
            .sender_avatar_with(account.id.clone(), address.into(), |_, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(synthetic_image(20)) })
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "unscoped legacy cache must not be read"
        );
        let current = update_route(&core, &account, json!({"kind":"inherit"}));
        assert!(current.revision > account.revision);
        let changed = core
            .sender_avatar_with(account.id.clone(), address.into(), |_, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(synthetic_image(30)) })
            })
            .await
            .unwrap()
            .unwrap();
        assert_ne!(loaded.data, changed.data);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let cached = core
            .sender_avatar_with(account.id.clone(), address.into(), |_, _| {
                panic!("same-revision positive cache should avoid network")
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cached, changed);
        core.avatars.clear();
        let disk = core
            .sender_avatar_with(account.id.clone(), address.into(), |_, _| {
                panic!("same-revision disk cache should avoid network")
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(disk, changed);
        let binding = AvatarBinding::read(&core.accounts().unwrap(), account.id.clone()).unwrap();
        let other_account = AvatarBinding {
            account_id: uuid::Uuid::new_v4().to_string(),
            ..binding.clone()
        };
        assert_ne!(
            binding.key("photo", address),
            other_account.key("photo", address)
        );
        let other_source = AvatarBinding {
            mailbox_source_id: uuid::Uuid::new_v4().to_string(),
            ..binding
        };
        assert_ne!(
            other_source.key("icon", "example.com"),
            other_account.key("icon", "example.com")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn late_network_result_after_route_change_is_not_returned_or_cached() {
        let (_directory, core, account) = fixture();
        let started = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        let task_core = core.clone();
        let task_account = account.id.clone();
        let task_started = started.clone();
        let task_finish = finish.clone();
        let pending = tokio::spawn(async move {
            task_core
                .sender_avatar_with(task_account, "sender@gmail.com".into(), move |_, _| {
                    let started = task_started.clone();
                    let finish = task_finish.clone();
                    Box::pin(async move {
                        started.notify_one();
                        finish.notified().await;
                        Ok(synthetic_image(50))
                    })
                })
                .await
        });
        started.notified().await;
        update_route(&core, &account, json!({"kind":"inherit"}));
        finish.notify_one();
        assert!(pending.await.unwrap().is_err());
        assert!(core.avatars.entries.lock().unwrap().is_empty());
        let old = AvatarBinding {
            account_id: account.id,
            mailbox_source_id: account.mailbox_source_id,
            revision: account.revision,
        };
        assert!(
            core.images
                .get(&old.key("photo", "sender@gmail.com"), FOUND_TTL)
                .is_none()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn temporary_network_errors_do_not_become_cached_missing_avatars() {
        let (_directory, core, account) = fixture();
        for failure in [
            ImageFetchError::Dns,
            ImageFetchError::Connect,
            ImageFetchError::Proxy,
            ImageFetchError::Tls,
            ImageFetchError::Timeout,
            ImageFetchError::HttpStatus,
        ] {
            assert!(
                core.sender_avatar_with(account.id.clone(), "sender@gmail.com".into(), |_, _| {
                    Box::pin(async move { Err(failure) })
                })
                .await
                .unwrap()
                .is_none()
            );
        }
        assert!(core.avatars.entries.lock().unwrap().is_empty());
        assert!(
            core.sender_avatar_with(account.id, "sender@gmail.com".into(), |_, _| {
                Box::pin(async { Ok(synthetic_image(60)) })
            })
            .await
            .unwrap()
            .is_some()
        );
    }

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
    fn cache_returns_only_successes_and_bounds_its_size() {
        let cache = AvatarCache::default();
        assert!(cache.get("a@b.c").is_none());
        let image = AvatarImage {
            mime_type: "image/png".into(),
            data: "AA==".into(),
            kind: AvatarKind::Photo,
        };
        cache.put("d@e.f".into(), image.clone());
        assert_eq!(cache.get("d@e.f"), Some(image.clone()));
        for index in 0..(MAX_ENTRIES + 5) {
            cache.put(format!("u{index}@x.io"), image.clone());
        }
        assert!(cache.entries.lock().unwrap().len() <= MAX_ENTRIES);
        cache.clear();
        assert!(cache.get("d@e.f").is_none());
    }
}
