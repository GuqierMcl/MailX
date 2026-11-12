//! Translating cached messages through a third-party service.
//!
//! Status never touches the network or downloads a body. A translation reads
//! the cached sanitized HTML, sends only its text segments (plus the subject)
//! to the service the user consented to, and caches the answers as wire
//! strings; they only become HTML through `translate::apply` in the isolated
//! body protocol. Service keys and the proxy password live in the
//! `SecretStore` and are never returned.
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::Serialize;
use tokio::sync::{Semaphore, watch};
use ts_rs::TS;

use super::{MailxCore, bodies::BodyBinding};
use crate::{
    AppError, ErrorCode,
    accounts::{AccountService, MAX_SECRET_BYTES},
    mail::{LOCAL_SENT_PREFIX, render},
    net::{NetRoute, ProxyRoute},
    secrets::{SecretStore, SecretValue},
    settings::{TranslationProviderId, TranslationRoute, TranslationSettings},
    storage::{
        bodies::{BodyIdentity, StoredBlob},
        translations::{NewTranslation, StoredTranslation, TranslationKey},
    },
    translate::{
        self, Detection, MAX_HTML_BYTES, Piece, SEGMENTER_VERSION, Segment, languages,
        provider::{self, Context, Limits, TranslateRequest, TranslateResponse},
        wire,
    },
};

/// Plain characters queued for one message; the rest stays untranslated.
const MAX_QUEUED_CHARS: usize = 50_000;
/// Translations running at once across the app.
const MAX_CONCURRENT: usize = 2;
const DETECTION_SAMPLE_CHARS: usize = 4000;
/// The walk never depends on the label text.
const FOLD_LABEL: &str = "x";
const TEST_SENTENCE: &str = "Hello, this is a test.";
const TEST_RESULT_CHARS: usize = 200;
const MAX_KEY_CHARS: usize = 512;
/// Cannot collide with the per-service names above.
const PROXY_PASSWORD_SECRET: &str = "mailx/translation/proxy-password";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslationKeyState {
    pub provider: TranslationProviderId,
    pub configured: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslationProxyPasswordState {
    pub configured: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslationTest {
    pub translated: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslationInfo {
    pub provider: TranslationProviderId,
    pub source_language: Option<String>,
    pub subject: Option<String>,
    /// Some text was left untranslated (size budget).
    pub partial: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslationStatus {
    pub email_id: String,
    /// Resolved: the explicit target, else the setting, with `auto` following the UI language.
    pub target_language: String,
    /// Base code from local detection; `None` when unknown.
    pub detected_language: Option<String>,
    pub detection_reliable: bool,
    pub suggested: bool,
    pub auto_translate: bool,
    /// Present when a current cached translation exists for the target and quote setting.
    pub translation: Option<TranslationInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslationCacheStats {
    #[ts(type = "number")]
    pub count: u64,
    #[ts(type = "number")]
    pub bytes: u64,
}

// ------------------------------------------------------------ in-flight work

struct Flight {
    id: u64,
    cancel: Arc<watch::Sender<bool>>,
}

/// Registered while a translation of one message runs. Dropping it frees the slot.
struct Registration {
    owner: Arc<Translations>,
    email_id: String,
    id: u64,
    cancel: Arc<watch::Sender<bool>>,
}

impl Registration {
    fn receiver(&self) -> watch::Receiver<bool> {
        self.cancel.subscribe()
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut flights = self
            .owner
            .flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if flights
            .get(&self.email_id)
            .is_some_and(|flight| flight.id == self.id)
        {
            flights.remove(&self.email_id);
        }
    }
}

/// Resolves once cancellation was requested.
async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    // The registration keeps the sender alive, so an error cannot happen; if it
    // somehow did, waiting forever is safer than cancelling the user's work.
    if receiver.wait_for(|cancelled| *cancelled).await.is_err() {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
pub(crate) type TestFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<TranslateResponse, ErrorCode>> + Send>,
>;

/// What a test backend sees of one provider request.
#[cfg(test)]
pub(crate) struct TestCall {
    pub provider: TranslationProviderId,
    pub texts: Vec<String>,
    pub target: String,
    pub region: String,
    pub proxy: Option<(String, u16)>,
    /// Proxy username and password, when the request would sign in.
    pub proxy_auth: Option<(String, String)>,
    pub key: String,
}

#[cfg(test)]
pub(crate) type TestBackend = Arc<dyn Fn(TestCall) -> TestFuture + Send + Sync>;

pub(super) struct Translations {
    secrets: Arc<dyn SecretStore>,
    flights: Mutex<HashMap<String, Flight>>,
    next_flight: AtomicU64,
    permits: Arc<Semaphore>,
    #[cfg(test)]
    backend: Mutex<Option<TestBackend>>,
}

impl Translations {
    pub(super) fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            secrets,
            flights: Mutex::default(),
            next_flight: AtomicU64::new(1),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            #[cfg(test)]
            backend: Mutex::default(),
        }
    }

    /// A newer translation of the same message cancels the older one.
    fn start(self: &Arc<Self>, email_id: &str) -> Registration {
        let id = self.next_flight.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(watch::channel(false).0);
        let mut flights = self.flights.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(old) = flights.insert(
            email_id.to_owned(),
            Flight {
                id,
                cancel: cancel.clone(),
            },
        ) {
            old.cancel.send_replace(true);
        }
        Registration {
            owner: self.clone(),
            email_id: email_id.to_owned(),
            id,
            cancel,
        }
    }

    fn cancel(&self, email_id: &str) {
        let flights = self.flights.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(flight) = flights.get(email_id) {
            flight.cancel.send_replace(true);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_backend(&self, backend: TestBackend) {
        *self.backend.lock().unwrap() = Some(backend);
    }

    /// The one place provider calls leave the app; tests replace it.
    async fn call(
        &self,
        provider: TranslationProviderId,
        key: &SecretValue,
        route: &NetRoute,
        settings: &TranslationSettings,
        request: &TranslateRequest<'_>,
    ) -> Result<TranslateResponse, ErrorCode> {
        #[cfg(test)]
        {
            let backend = self.backend.lock().unwrap().clone();
            if let Some(backend) = backend {
                return backend(TestCall {
                    provider,
                    texts: request.texts.to_vec(),
                    target: request.target.to_owned(),
                    region: settings.microsoft_region.clone(),
                    proxy: route.proxy.as_ref().map(|p| (p.host.clone(), p.port)),
                    proxy_auth: route.proxy.as_ref().and_then(|p| {
                        let (user, password) = p.auth.as_ref()?;
                        Some((user.clone(), password.expose_secret().to_owned()))
                    }),
                    key: key.expose_secret().to_owned(),
                })
                .await;
            }
        }
        let context = Context {
            key,
            microsoft_region: &settings.microsoft_region,
            route,
            #[cfg(test)]
            base: None,
        };
        provider::translate(provider, &context, request).await
    }
}

/// Who a request goes to and how it gets there.
struct Service<'a> {
    provider: TranslationProviderId,
    key: &'a SecretValue,
    route: &'a NetRoute,
}

fn secret_name(provider: TranslationProviderId) -> String {
    format!("mailx/translation/{}", provider.as_str())
}

/// The mail account's own proxy is never used here. Signing in needs a
/// username; a password stored without one is ignored.
fn net_route(route: &TranslationRoute, password: Option<SecretValue>) -> NetRoute {
    match route {
        TranslationRoute::Direct => NetRoute::default(),
        TranslationRoute::Proxy {
            proxy_kind,
            host,
            port,
            username,
        } => NetRoute {
            proxy: Some(ProxyRoute {
                kind: *proxy_kind,
                host: host.clone(),
                port: *port,
                auth: (!username.is_empty())
                    .then(|| (username.clone(), password.unwrap_or_else(empty_secret))),
            }),
            image_proxy_dns: false,
        },
    }
}

fn empty_secret() -> SecretValue {
    SecretValue::new(String::new())
}

/// Not trimmed: a password may start or end with a space.
fn valid_proxy_password(password: &SecretValue) -> bool {
    let text = password.expose_secret();
    !text.is_empty() && text.len() <= MAX_SECRET_BYTES && !text.chars().any(char::is_control)
}

fn internal() -> AppError {
    AppError::new(ErrorCode::Internal)
}

fn resolve_target(
    settings: &TranslationSettings,
    ui_language: &str,
    explicit: Option<&str>,
) -> Result<String, AppError> {
    match explicit {
        Some(target) if languages::is_target(target) => Ok(target.to_owned()),
        Some(_) => Err(AppError::validation("target")),
        None if settings.target_language == "auto" => {
            Ok(languages::for_ui_language(ui_language).to_owned())
        }
        None => Ok(settings.target_language.clone()),
    }
}

fn valid_key(key: &SecretValue) -> Option<SecretValue> {
    let key = key.expose_secret().trim();
    (!key.is_empty() && key.chars().count() <= MAX_KEY_CHARS && !key.chars().any(char::is_control))
        .then(|| SecretValue::new(key.to_owned()))
}

// ------------------------------------------------------------ pure planning

/// Which wire strings to send and where each segment's answer goes.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    /// Distinct wire strings, in first-use order.
    unique: Vec<String>,
    /// Per segment: index into `unique`, or `None` to leave it untranslated.
    slots: Vec<Option<usize>>,
    partial: bool,
}

/// Identical segments are sent once. Unique text beyond the budget, or too big
/// for one request, is left alone and makes the result partial; a repeat of
/// something already queued costs nothing and is always translated.
fn plan(segments: &[Segment], limits: Limits) -> Plan {
    let mut unique: Vec<String> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut slots = Vec::with_capacity(segments.len());
    let mut queued = 0usize;
    let mut partial = false;
    for segment in segments {
        let encoded = wire::encode(segment);
        if let Some(&index) = seen.get(&encoded) {
            slots.push(Some(index));
            continue;
        }
        if queued >= MAX_QUEUED_CHARS || encoded.len() > limits.max_bytes {
            slots.push(None);
            partial = true;
            continue;
        }
        queued += segment.char_count();
        seen.insert(encoded.clone(), unique.len());
        slots.push(Some(unique.len()));
        unique.push(encoded);
    }
    Plan {
        unique,
        slots,
        partial,
    }
}

/// Consecutive ranges of `items`, each within the request limits.
fn batches(items: &[String], limits: Limits) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut bytes = 0;
    for (index, item) in items.iter().enumerate() {
        let size = item.len();
        if index > start && (index - start >= limits.max_items || bytes + size > limits.max_bytes) {
            out.push(start..index);
            start = index;
            bytes = 0;
        }
        bytes += size;
    }
    if start < items.len() {
        out.push(start..items.len());
    }
    out
}

fn text_wire(text: &str) -> String {
    wire::encode(&Segment::new(vec![Piece::Text(text.to_owned())]))
}

/// Plain text of a service answer to a `text_wire` request.
fn wire_text(answer: &str) -> String {
    wire::decode(answer)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Text(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn detect(html: &str) -> Option<Detection> {
    if html.len() > MAX_HTML_BYTES {
        return None;
    }
    let folded = render::fold_quotes(html, FOLD_LABEL);
    let segments = translate::extract(&folded, false).ok()?;
    translate::detect_language(&translate::sample_text(&segments, DETECTION_SAMPLE_CHARS))
}

struct StatusInput<'a> {
    email_id: &'a str,
    settings: &'a TranslationSettings,
    target: String,
    provider_ready: bool,
    detection: Option<Detection>,
    translation: Option<StoredTranslation>,
}

fn build_status(input: StatusInput<'_>) -> TranslationStatus {
    let StatusInput {
        email_id,
        settings,
        target,
        provider_ready,
        detection,
        translation,
    } = input;
    // Local Sent copies cannot be cached, so they are never offered.
    let offerable = provider_ready && !email_id.starts_with(LOCAL_SENT_PREFIX);
    let detected = detection
        .as_ref()
        .map(|d| languages::base(&d.language).to_ascii_lowercase());
    let reliable = detection.as_ref().is_some_and(|d| d.reliable);
    let foreign = reliable
        && detected
            .as_deref()
            .is_some_and(|code| !languages::same_language(code, &target));
    let listed = |list: &[String]| {
        detected
            .as_deref()
            .is_some_and(|code| list.iter().any(|entry| entry == code))
    };
    let consented = settings.provider.is_some() && settings.consented_provider == settings.provider;
    TranslationStatus {
        email_id: email_id.to_owned(),
        target_language: target,
        detected_language: detected.clone(),
        detection_reliable: reliable,
        suggested: offerable && settings.suggest && foreign && !listed(&settings.never_suggest),
        auto_translate: offerable && consented && foreign && listed(&settings.always_translate),
        translation: translation.map(|row| TranslationInfo {
            provider: row.provider,
            source_language: row.source_language,
            subject: row.subject,
            partial: row.partial,
        }),
    }
}

// ----------------------------------------------------------------- service

/// What a translation reads from the cached body, in one locked look.
struct Source {
    identity: BodyIdentity,
    raw: StoredBlob,
    sanitizer_version: u32,
    html: String,
    subject: Option<String>,
}

/// `None` when the body is not cached (or cached by an older sanitizer).
fn read_source(service: &AccountService, email_id: &str) -> Result<Option<Source>, AppError> {
    let identity = service.body_identity(email_id)?;
    let Some(body) = service.body_cache(&identity)? else {
        return Ok(None);
    };
    if body.sanitizer_version != render::SANITIZER_VERSION {
        return Ok(None);
    }
    let Some(html) = body.rendered.sanitized_html else {
        return Ok(None);
    };
    Ok(Some(Source {
        subject: service.translation_subject(email_id)?,
        identity,
        raw: body.raw,
        sanitizer_version: body.sanitizer_version,
        html,
    }))
}

impl MailxCore {
    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&MailxCore) -> Result<T, AppError> + Send + 'static,
    ) -> Result<T, AppError> {
        let core = self.clone();
        tokio::task::spawn_blocking(move || work(&core))
            .await
            .map_err(|_| internal())?
    }

    /// What a request to `provider` signs with: the saved key, or nothing for
    /// a keyless service. A keyed service without a key is not configured.
    async fn translation_key(
        &self,
        provider: TranslationProviderId,
    ) -> Result<SecretValue, AppError> {
        if !provider.needs_key() {
            return Ok(empty_secret());
        }
        let secrets = self.translation.secrets.clone();
        self.blocking(move |_| Ok(secrets.get(&secret_name(provider))?))
            .await?
            .ok_or_else(not_configured)
    }

    /// Builds the route for one request. A secret store that cannot be read
    /// while a username is set fails the request; it never goes out unsigned.
    async fn translation_route(&self, route: &TranslationRoute) -> Result<NetRoute, AppError> {
        let needs_password =
            matches!(route, TranslationRoute::Proxy { username, .. } if !username.is_empty());
        let password = if needs_password {
            let secrets = self.translation.secrets.clone();
            self.blocking(move |_| Ok(secrets.get(PROXY_PASSWORD_SECRET)?))
                .await?
        } else {
            None
        };
        Ok(net_route(route, password))
    }

    async fn proxy_password_state(&self) -> Result<TranslationProxyPasswordState, AppError> {
        let secrets = self.translation.secrets.clone();
        self.blocking(move |_| {
            Ok(TranslationProxyPasswordState {
                configured: secrets.get(PROXY_PASSWORD_SECRET)?.is_some(),
            })
        })
        .await
    }

    /// Stores the proxy password. It is never read back out.
    pub async fn set_translation_proxy_password(
        &self,
        password: SecretValue,
    ) -> Result<TranslationProxyPasswordState, AppError> {
        if !valid_proxy_password(&password) {
            return Err(AppError::validation("translation.route.password"));
        }
        let secrets = self.translation.secrets.clone();
        self.blocking(move |_| Ok(secrets.set(PROXY_PASSWORD_SECRET, &password)?))
            .await?;
        self.proxy_password_state().await
    }

    pub async fn remove_translation_proxy_password(
        &self,
    ) -> Result<TranslationProxyPasswordState, AppError> {
        let secrets = self.translation.secrets.clone();
        self.blocking(move |_| Ok(secrets.delete(PROXY_PASSWORD_SECRET)?))
            .await?;
        self.proxy_password_state().await
    }

    pub async fn translation_proxy_password(
        &self,
    ) -> Result<TranslationProxyPasswordState, AppError> {
        self.proxy_password_state().await
    }

    /// Stores the key for a service. The key is trimmed; it is never read back out.
    pub async fn set_translation_key(
        &self,
        provider: TranslationProviderId,
        key: SecretValue,
    ) -> Result<(), AppError> {
        if !provider.needs_key() {
            return Err(AppError::validation("translation.provider"));
        }
        let key = valid_key(&key).ok_or_else(|| AppError::validation("translation.key"))?;
        let secrets = self.translation.secrets.clone();
        self.blocking(move |_| Ok(secrets.set(&secret_name(provider), &key)?))
            .await
    }

    pub async fn remove_translation_key(
        &self,
        provider: TranslationProviderId,
    ) -> Result<(), AppError> {
        if !provider.needs_key() {
            return Err(AppError::validation("translation.provider"));
        }
        let secrets = self.translation.secrets.clone();
        self.blocking(move |_| Ok(secrets.delete(&secret_name(provider))?))
            .await
    }

    pub async fn translation_keys(&self) -> Result<Vec<TranslationKeyState>, AppError> {
        let secrets = self.translation.secrets.clone();
        self.blocking(move |_| {
            TranslationProviderId::ALL
                .into_iter()
                .filter(|provider| provider.needs_key())
                .map(|provider| {
                    Ok(TranslationKeyState {
                        provider,
                        configured: secrets.get(&secret_name(provider))?.is_some(),
                    })
                })
                .collect()
        })
        .await
    }

    /// Translates a fixed English sentence, to check key (if the service has one), region and route. It
    /// carries no mail text, so it needs no consent. With the target set to
    /// `auto` there is no UI language to follow, so Simplified Chinese is used.
    pub async fn test_translation(
        &self,
        settings: TranslationSettings,
    ) -> Result<TranslationTest, AppError> {
        let settings = settings.normalized()?;
        let provider = settings.provider.ok_or_else(not_configured)?;
        let key = self.translation_key(provider).await?;
        let target = match settings.target_language.as_str() {
            "auto" => "zh-Hans",
            explicit => explicit,
        };
        let route = self.translation_route(&settings.route).await?;
        let texts = [text_wire(TEST_SENTENCE)];
        let response = self
            .translation
            .call(
                provider,
                &key,
                &route,
                &settings,
                &TranslateRequest {
                    texts: &texts,
                    target,
                },
            )
            .await
            .map_err(AppError::new)?;
        let answer = response
            .texts
            .first()
            .map(|text| wire_text(text))
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| AppError::new(ErrorCode::TranslationInvalidResponse))?;
        Ok(TranslationTest {
            translated: answer.chars().take(TEST_RESULT_CHARS).collect(),
        })
    }

    /// Local only: no network, no body download. Without a cached body the
    /// language is unknown.
    pub async fn translation_status(
        &self,
        email_id: String,
        settings: TranslationSettings,
        ui_language: String,
    ) -> Result<TranslationStatus, AppError> {
        let settings = settings.normalized()?;
        let target = resolve_target(&settings, &ui_language, None)?;
        self.status_for(email_id, settings, target).await
    }

    async fn status_for(
        &self,
        email_id: String,
        settings: TranslationSettings,
        target: String,
    ) -> Result<TranslationStatus, AppError> {
        let secrets = self.translation.secrets.clone();
        let (provider_ready, html, translation) = {
            let (id, settings, target) = (email_id.clone(), settings.clone(), target.clone());
            self.blocking(move |core| {
                let ready = match settings.provider {
                    Some(provider) if !provider.needs_key() => true,
                    Some(provider) => secrets.get(&secret_name(provider))?.is_some(),
                    None => false,
                };
                let service = core.accounts()?;
                let Some(source) = read_source(&service, &id)? else {
                    return Ok((ready, None, None));
                };
                let row = service.translation_get(&TranslationKey {
                    message_id: &id,
                    target_language: &target,
                    include_quotes: settings.include_quotes,
                    raw_sha256: &source.raw.sha256,
                    sanitizer_version: source.sanitizer_version,
                    segmenter_version: SEGMENTER_VERSION,
                })?;
                Ok((ready, Some(source.html), row))
            })
            .await?
        };
        // Detection is CPU work and runs without the store lock.
        let detection = match html {
            Some(html) => tokio::task::spawn_blocking(move || detect(&html))
                .await
                .map_err(|_| internal())?,
            None => None,
        };
        Ok(build_status(StatusInput {
            email_id: &email_id,
            settings: &settings,
            target,
            provider_ready,
            detection,
            translation,
        }))
    }

    /// Translates the cached body and stores the result. Checks run in the
    /// documented order; a second call for the same message cancels the first.
    pub async fn translate_email(
        &self,
        email_id: String,
        settings: TranslationSettings,
        ui_language: String,
        target: Option<String>,
    ) -> Result<TranslationStatus, AppError> {
        let settings = settings.normalized()?;
        let provider = settings.provider.ok_or_else(not_configured)?;
        let key = self.translation_key(provider).await?;
        // Checked here so that no host can skip the user's agreement.
        if settings.consented_provider != Some(provider) {
            return Err(AppError::new(ErrorCode::TranslationConsentRequired));
        }
        let target = resolve_target(&settings, &ui_language, target.as_deref())?;
        let route = self.translation_route(&settings.route).await?;
        let registration = self.translation.start(&email_id);
        let mut cancel = registration.receiver();
        let work = self.translate_inner(
            &email_id,
            &settings,
            Service {
                provider,
                key: &key,
                route: &route,
            },
            &target,
            registration.receiver(),
        );
        tokio::select! {
            biased;
            () = cancelled(&mut cancel) => return Err(AppError::new(ErrorCode::TranslationCancelled)),
            done = work => done?,
        }
        drop(registration);
        self.status_for(email_id, settings, target).await
    }

    async fn translate_inner(
        &self,
        email_id: &str,
        settings: &TranslationSettings,
        service: Service<'_>,
        target: &str,
        cancel: watch::Receiver<bool>,
    ) -> Result<(), AppError> {
        let Service {
            provider,
            key,
            route,
        } = service;
        // Local Sent copies have no message row to hang a cache entry on.
        if email_id.starts_with(LOCAL_SENT_PREFIX) {
            return Err(AppError::validation("emailId"));
        }
        let _permit = self
            .translation
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| internal())?;
        let id = email_id.to_owned();
        let source = self
            .blocking(move |core| {
                let service = core.accounts()?;
                read_source(&service, &id)
            })
            .await?
            .ok_or_else(|| AppError::new(ErrorCode::BodyNotCached))?;
        let too_large = || AppError::new(ErrorCode::TranslationTooLarge);
        if source.html.len() > MAX_HTML_BYTES {
            return Err(too_large());
        }
        let include_quotes = settings.include_quotes;
        let html = source.html.clone();
        // Parsing a large message is CPU work; keep it off the async threads.
        let translate::Extraction {
            segments,
            truncated,
        } = tokio::task::spawn_blocking(move || {
            translate::extract_checked(&render::fold_quotes(&html, FOLD_LABEL), include_quotes)
        })
        .await
        .map_err(|_| internal())?
        .map_err(|_| too_large())?;

        let limits = provider::limits(provider);
        let Plan {
            unique,
            slots,
            partial,
        } = plan(&segments, limits);
        // Text past the segment cap is never sent, so it is as partial as a spent budget.
        let partial = partial || truncated;
        let subject = source
            .subject
            .as_deref()
            .filter(|subject| !subject.trim().is_empty())
            .map(text_wire)
            .filter(|wire| wire.len() <= limits.max_bytes);
        let has_subject = subject.is_some();
        let mut items: Vec<String> = subject.into_iter().collect();
        items.extend(unique);

        let mut answers: Vec<String> = Vec::with_capacity(items.len());
        let mut detected_source = None;
        for range in batches(&items, limits) {
            let response = self
                .translation
                .call(
                    provider,
                    key,
                    route,
                    settings,
                    &TranslateRequest {
                        texts: &items[range],
                        target,
                    },
                )
                .await
                .map_err(AppError::new)?;
            detected_source = detected_source.or(response.detected_source);
            answers.extend(response.texts);
        }
        if answers.len() != items.len() {
            return Err(AppError::new(ErrorCode::TranslationInvalidResponse));
        }
        let offset = usize::from(has_subject);
        let translated_subject = has_subject
            .then(|| wire_text(&answers[0]))
            .filter(|text| !text.trim().is_empty());
        let stored: Vec<Option<String>> = slots
            .iter()
            .map(|slot| slot.map(|index| answers[offset + index].clone()))
            .collect();

        let (message_id, target) = (source.identity.message_id.clone(), target.to_owned());
        let expected = (source.raw.clone(), source.sanitizer_version);
        let written = self
            .blocking(move |core| {
                let mut service = core.accounts()?;
                // A cancel that raced the last request must not write.
                if *cancel.borrow() {
                    return Err(AppError::new(ErrorCode::TranslationCancelled));
                }
                service.translation_put(
                    &source.identity,
                    &expected,
                    &NewTranslation {
                        message_id: &message_id,
                        target_language: &target,
                        include_quotes,
                        raw_sha256: &expected.0.sha256,
                        sanitizer_version: expected.1,
                        segmenter_version: SEGMENTER_VERSION,
                        provider,
                        source_language: detected_source.as_deref(),
                        subject: translated_subject.as_deref(),
                        segments: &stored,
                        partial,
                    },
                )
            })
            .await?;
        if !written {
            // The body changed while the service was working.
            return Err(AppError::new(ErrorCode::BodyNotCached));
        }
        Ok(())
    }

    /// Asks a running translation of this message to stop; it returns `TranslationCancelled`.
    pub fn cancel_translation(&self, email_id: &str) {
        self.translation.cancel(email_id);
    }

    /// Removes the cached translations of one message, whatever the target.
    pub async fn clear_translation(&self, email_id: String) -> Result<(), AppError> {
        self.blocking(move |core| core.accounts()?.translation_delete_message(&email_id))
            .await
    }

    pub async fn clear_translations(&self) -> Result<(), AppError> {
        self.blocking(|core| core.accounts()?.translation_delete_all())
            .await
    }

    pub async fn translation_cache_stats(&self) -> Result<TranslationCacheStats, AppError> {
        self.blocking(|core| {
            let stats = core.accounts()?.translation_stats()?;
            Ok(TranslationCacheStats {
                count: stats.count,
                bytes: stats.bytes,
            })
        })
        .await
    }

    /// For the host's body protocol: the wire strings for the document this
    /// binding serves, or `None` when there is no current translation.
    pub async fn body_translation(
        &self,
        binding: &BodyBinding,
        target: String,
        include_quotes: bool,
    ) -> Result<Option<Vec<Option<String>>>, AppError> {
        if !languages::is_target(&target) {
            return Err(AppError::validation("target"));
        }
        let binding = binding.clone();
        self.blocking(move |core| {
            let service = core.accounts()?;
            super::bodies::check(&service, &binding)?;
            Ok(service
                .translation_get(&TranslationKey {
                    message_id: &binding.identity.message_id,
                    target_language: &target,
                    include_quotes,
                    raw_sha256: &binding.raw.sha256,
                    sanitizer_version: binding.sanitizer_version,
                    segmenter_version: SEGMENTER_VERSION,
                })?
                .map(|row| row.segments))
        })
        .await
    }
}

fn not_configured() -> AppError {
    AppError::new(ErrorCode::TranslationNotConfigured)
}

#[cfg(test)]
mod tests;
