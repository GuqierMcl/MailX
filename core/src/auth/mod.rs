//! Host-independent OAuth primitives. Registrations are trusted application code,
//! never values supplied by a mailbox, MCP caller, or discovery document.
use crate::{
    net::NetRoute,
    secrets::{SecretStore, SecretValue},
};
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;

pub(crate) mod credential;
pub(crate) mod id_token;
pub(crate) mod loopback;
mod transport;

/// Stable diagnostics intentionally exclude URLs, tokens and server response text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuthError {
    UnsupportedProvider,
    InvalidRegistration,
    InvalidCallback,
    ExpiredSession,
    Random,
    Network,
    Proxy,
    Tls,
    RateLimited,
    ServerUnavailable,
    Timeout,
    InvalidResponse,
    AuthorizationDenied,
    ReauthenticationRequired,
    Storage,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenTarget {
    pub resource: Option<String>,
    pub scopes: Vec<String>,
}
impl TokenTarget {
    pub(crate) fn new(resource: Option<String>, mut scopes: Vec<String>) -> Self {
        scopes.sort();
        scopes.dedup();
        Self { resource, scopes }
    }
}
/// Reviewed mail destinations and SASL login rules for one provider adapter.
/// OAuth authorization alone never permits forwarding its bearer to arbitrary
/// account-supplied mail servers or login identities.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MailEndpointPolicy {
    pub imap: MailProtocolPolicy,
    pub smtp: MailProtocolPolicy,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MailProtocolPolicy {
    pub host: String,
    pub port: u16,
    pub tls: crate::accounts::types::TlsMode,
    pub login: MailLoginPolicy,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "username", rename_all = "camelCase")]
pub(crate) enum MailLoginPolicy {
    Mailbox,
    Fixed(String),
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderRegistration {
    pub id: String,
    pub client_id: String,
    pub registration_id: String,
    pub mail_policy: MailEndpointPolicy,
    pub authorization_endpoint: Url,
    pub token_endpoint: Url,
    pub scopes: Vec<String>,
    pub resource: Option<String>,
    #[serde(default)]
    pub allowed_targets: Vec<TokenTarget>,
    /// Path of the redirect URI registered with the provider, such as
    /// `/callback`. Providers ignore only the port of a loopback redirect, so
    /// the path is fixed by the registration. Empty means no path at all.
    #[serde(default)]
    pub redirect_path: String,
    /// Static parameters appended to the authorization request. They come from
    /// reviewed code only and may not replace a parameter the flow sets itself.
    #[serde(default)]
    pub extra_authorize_params: Vec<(String, String)>,
    /// Whether the authorization request carries the account address as
    /// `login_hint`, so the provider can preselect the account.
    #[serde(default)]
    pub login_hint: bool,
}
/// Authorization parameters the flow sets itself.
const RESERVED_AUTHORIZE_PARAMS: &[&str] = &[
    "response_type",
    "client_id",
    "redirect_uri",
    "scope",
    "state",
    "code_challenge",
    "code_challenge_method",
    "resource",
    "nonce",
    "login_hint",
];
impl ProviderRegistration {
    /// The exact `redirect_uri` text for a loopback listener on `port`. The
    /// same string goes into the authorization and token requests; an empty
    /// path yields no trailing slash.
    pub(crate) fn redirect_uri(&self, port: u16) -> String {
        format!("http://127.0.0.1:{port}{}", self.redirect_path)
    }
    pub(crate) fn validate_mail_target(
        &self,
        mailbox: &str,
        imap: &crate::accounts::types::ConnectionConfig,
        smtp: &crate::accounts::types::ConnectionConfig,
    ) -> Result<(), OAuthError> {
        let mailbox = mailbox.trim().to_ascii_lowercase();
        if mailbox.is_empty() || mailbox.chars().any(char::is_control) {
            return Err(OAuthError::InvalidRegistration);
        }
        for (policy, connection) in [
            (&self.mail_policy.imap, imap),
            (&self.mail_policy.smtp, smtp),
        ] {
            let login_matches = match &policy.login {
                MailLoginPolicy::Mailbox => connection.username.eq_ignore_ascii_case(&mailbox),
                MailLoginPolicy::Fixed(username) => {
                    !username.is_empty() && connection.username == *username
                }
            };
            if connection.host != policy.host
                || connection.port != policy.port
                || connection.tls != policy.tls
                || !login_matches
            {
                return Err(OAuthError::InvalidRegistration);
            }
        }
        Ok(())
    }
    /// OpenID Connect is requested, so the flow sends a nonce and the verifier
    /// can rely on the `id_token`.
    fn requests_openid(&self) -> bool {
        self.scopes.iter().any(|s| s == "openid")
    }
    fn default_target(&self) -> TokenTarget {
        TokenTarget::new(self.resource.clone(), self.scopes.clone())
    }
    fn permits(&self, target: &TokenTarget) -> bool {
        *target == self.default_target()
            || self
                .allowed_targets
                .iter()
                .any(|t| TokenTarget::new(t.resource.clone(), t.scopes.clone()) == *target)
    }

    pub(crate) fn validate(&self) -> Result<(), OAuthError> {
        if self.id.is_empty()
            || self.client_id.is_empty()
            || self.registration_id.is_empty()
            || self.scopes.is_empty()
        {
            return Err(OAuthError::InvalidRegistration);
        }
        for policy in [&self.mail_policy.imap, &self.mail_policy.smtp] {
            if policy.host.is_empty()
                || policy.host.len() > 253
                || policy
                    .host
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
                || policy.port == 0
                || matches!(&policy.login, MailLoginPolicy::Fixed(name) if name.is_empty() || name.chars().any(char::is_control))
            {
                return Err(OAuthError::InvalidRegistration);
            }
        }
        for endpoint in [&self.authorization_endpoint, &self.token_endpoint] {
            if endpoint.scheme() != "https"
                || endpoint.host_str().is_none()
                || !endpoint.username().is_empty()
                || endpoint.password().is_some()
                || endpoint.fragment().is_some()
                || endpoint.query().is_some()
            {
                #[cfg(test)]
                if endpoint.scheme() == "http"
                    && endpoint.host_str() == Some("127.0.0.1")
                    && endpoint.query().is_none()
                    && endpoint.fragment().is_none()
                    && endpoint.username().is_empty()
                    && endpoint.password().is_none()
                {
                    continue;
                }
                return Err(OAuthError::InvalidRegistration);
            }
        }
        if !(self.redirect_path.is_empty()
            || self.redirect_path.len() <= 128
                && self.redirect_path.starts_with('/')
                && !self.redirect_path.starts_with("//")
                && !self
                    .redirect_path
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '?' | '#' | '\\')))
            || self.extra_authorize_params.len() > 8
            || self.extra_authorize_params.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 64
                    || value.len() > 512
                    || RESERVED_AUTHORIZE_PARAMS.contains(&key.as_str())
                    || key.chars().any(char::is_control)
                    || value.chars().any(char::is_control)
            })
        {
            return Err(OAuthError::InvalidRegistration);
        }
        if self
            .scopes
            .iter()
            .any(|s| s.is_empty() || s.chars().any(char::is_whitespace))
            || self
                .resource
                .as_ref()
                .is_some_and(|r| r.is_empty() || r.len() > 4096 || r.chars().any(char::is_control))
        {
            return Err(OAuthError::InvalidRegistration);
        }
        for target in
            std::iter::once(self.default_target()).chain(self.allowed_targets.iter().cloned())
        {
            if target.scopes.is_empty()
                || target.scopes.len() > 64
                || target
                    .scopes
                    .iter()
                    .any(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_whitespace))
                || target.resource.as_ref().is_some_and(|r| {
                    r.is_empty() || r.len() > 4096 || r.chars().any(char::is_control)
                })
            {
                return Err(OAuthError::InvalidRegistration);
            }
        }
        Ok(())
    }
}

/// One reviewed OAuth provider entry. Several entries may share a client ID
/// and differ in their endpoints, so an entry is identified by the id of its
/// registration, never by the client ID. Implementations live in
/// `crate::providers`; this module stays provider-neutral.
pub(crate) trait OAuthProvider: Send + Sync {
    fn registration(&self) -> &ProviderRegistration;
    fn identity_verifier(&self) -> Arc<dyn IdentityVerifier>;
}
/// What one token exchange produced, for identity checks. The `id_token` is
/// held only for the duration of verification and is never persisted or logged.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Read by provider identity verifiers, which are added with the first provider"
    )
)]
pub(crate) struct IdentityEvidence<'a> {
    pub access_token: &'a SecretValue,
    pub id_token: Option<&'a SecretValue>,
    /// The nonce sent in the authorization request, when it carried one.
    pub nonce: Option<&'a str>,
    pub client_id: &'a str,
}
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Called by provider identity verifiers, which are added with the first provider"
    )
)]
impl IdentityEvidence<'_> {
    /// Claims of the exchange's `id_token`, checked against the client and nonce.
    pub(crate) fn id_token_claims(&self) -> Result<id_token::IdTokenClaims, OAuthError> {
        let token = self.id_token.ok_or(OAuthError::InvalidResponse)?;
        id_token::parse_id_token(token, self.client_id, self.nonce)
    }
}
/// Only reviewed provider adapters implement this boundary. A token response
/// alone cannot prove which mailbox the user authorized.
pub(crate) trait IdentityVerifier: Send + Sync {
    fn verify<'a>(
        &'a self,
        evidence: &'a IdentityEvidence<'a>,
        route: &'a NetRoute,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<VerifiedIdentity, OAuthError>> + Send + 'a>,
    >;
}
/// The issuer is whatever the verifier established for this grant. With
/// multi-tenant endpoints it differs per tenant, so it is not a registration
/// constant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedIdentity {
    pub issuer: String,
    pub subject: String,
    pub mailbox: String,
}
pub(crate) struct VerifiedAuthorization {
    pub(crate) binding: GrantBinding,
    pub(crate) tokens: TokenSet,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct GrantBinding {
    pub provider_id: String,
    /// Verified for this grant; never taken from the registration.
    pub issuer: String,
    pub client_registration_id: String,
    pub client_id: String,
    pub subject: String,
    pub mailbox: String,
    pub grant_id: String,
    pub generation: u64,
    pub resource: Option<String>,
    pub scopes: Vec<String>,
}
/// Consumed on every callback attempt, including malformed or mismatched state.
pub(crate) struct OAuthSession {
    provider: ProviderRegistration,
    redirect: Url,
    redirect_uri: String,
    nonce: Option<String>,
    state: oauth2::CsrfToken,
    verifier: SecretValue,
    deadline: Instant,
}
impl OAuthSession {
    /// `redirect_uri` is the exact text from the loopback listener. `login_hint`
    /// is the validated account address; it is sent only when the registration
    /// opts in.
    pub(crate) fn begin(
        provider: ProviderRegistration,
        redirect_uri: &str,
        login_hint: Option<&str>,
    ) -> Result<(Self, Url), OAuthError> {
        provider.validate()?;
        let redirect = Url::parse(redirect_uri).map_err(|_| OAuthError::InvalidRegistration)?;
        if redirect.scheme() != "http"
            || redirect.host_str() != Some("127.0.0.1")
            || redirect.port().is_none()
            || !redirect.username().is_empty()
            || redirect.password().is_some()
            || redirect.query().is_some()
            || redirect.fragment().is_some()
            || redirect_uri != provider.redirect_uri(redirect.port().unwrap_or_default())
        {
            return Err(OAuthError::InvalidRegistration);
        }
        let state = oauth2::CsrfToken::new_random();
        let (challenge, verifier) = oauth2::PkceCodeChallenge::new_random_sha256();
        let challenge = challenge.as_str().to_owned();
        let verifier = SecretValue::new(verifier.secret().clone());
        let mut authorization = provider.authorization_endpoint.clone();
        authorization.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", &provider.client_id),
            ("redirect_uri", redirect_uri),
            ("scope", &provider.scopes.join(" ")),
            ("state", state.secret()),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ]);
        if let Some(resource) = &provider.resource {
            authorization
                .query_pairs_mut()
                .append_pair("resource", resource);
        }
        let nonce = provider
            .requests_openid()
            .then(|| oauth2::CsrfToken::new_random().secret().clone());
        if let Some(nonce) = &nonce {
            authorization.query_pairs_mut().append_pair("nonce", nonce);
        }
        if provider.login_hint
            && let Some(hint) = login_hint
        {
            if hint.is_empty() || hint.len() > 320 || hint.chars().any(char::is_control) {
                return Err(OAuthError::InvalidRegistration);
            }
            authorization
                .query_pairs_mut()
                .append_pair("login_hint", hint);
        }
        authorization
            .query_pairs_mut()
            .extend_pairs(&provider.extra_authorize_params);
        Ok((
            Self {
                provider,
                redirect,
                redirect_uri: redirect_uri.to_owned(),
                nonce,
                state,
                verifier,
                deadline: Instant::now() + Duration::from_secs(300),
            },
            authorization,
        ))
    }
    pub(crate) async fn complete(
        self,
        callback: &Url,
        route: &NetRoute,
        verifier: &dyn IdentityVerifier,
        expected_mailbox: &str,
    ) -> Result<VerifiedAuthorization, OAuthError> {
        if Instant::now() >= self.deadline {
            return Err(OAuthError::ExpiredSession);
        }
        if callback.origin() != self.redirect.origin()
            || callback.path() != self.redirect.path()
            || callback.fragment().is_some()
            || !callback.username().is_empty()
            || callback.password().is_some()
        {
            return Err(OAuthError::InvalidCallback);
        }
        let pairs: Vec<_> = callback.query_pairs().collect();
        let single = |key: &str| -> Result<Option<String>, OAuthError> {
            let values: Vec<_> = pairs.iter().filter(|(k, _)| k == key).collect();
            if values.len() > 1 {
                return Err(OAuthError::InvalidCallback);
            }
            Ok(values.first().map(|(_, v)| v.to_string()))
        };
        let received_state = single("state")?.ok_or(OAuthError::InvalidCallback)?;
        if oauth2::CsrfToken::new(received_state) != self.state {
            return Err(OAuthError::InvalidCallback);
        }
        if single("error")?.is_some() {
            return Err(OAuthError::AuthorizationDenied);
        }
        let code = single("code")?
            .filter(|s| !s.is_empty() && s.len() <= 8192)
            .ok_or(OAuthError::InvalidCallback)?;
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            if let Some(resource) = &self.provider.resource {
                form.append_pair("resource", resource);
            }
            form.extend_pairs([
                ("grant_type", "authorization_code"),
                ("client_id", &self.provider.client_id),
                ("code", &code),
                ("redirect_uri", &self.redirect_uri),
                ("code_verifier", self.verifier.expose_secret()),
            ])
            .finish()
        };
        let transport::ExchangeResponse { tokens, id_token } = transport::exchange(
            route,
            &self.provider.token_endpoint,
            body,
            None,
            &self.provider.scopes,
        )
        .await?;
        if tokens.refresh_token.is_none() {
            return Err(OAuthError::InvalidResponse);
        }
        let evidence = IdentityEvidence {
            access_token: &tokens.access_token,
            id_token: id_token.as_ref(),
            nonce: self.nonce.as_deref(),
            client_id: &self.provider.client_id,
        };
        let identity =
            tokio::time::timeout(Duration::from_secs(30), verifier.verify(&evidence, route))
                .await
                .map_err(|_| OAuthError::Timeout)??;
        drop(id_token);
        if identity.issuer.is_empty()
            || identity.issuer.len() > 2048
            || identity.issuer.chars().any(char::is_control)
            || identity.subject.is_empty()
            || identity.subject.len() > 4096
            || identity.subject.chars().any(char::is_control)
            || identity.mailbox.is_empty()
            || identity.mailbox.len() > 512
            || identity.mailbox.chars().any(char::is_control)
            || !identity.mailbox.eq_ignore_ascii_case(expected_mailbox)
        {
            return Err(OAuthError::InvalidResponse);
        }
        Ok(VerifiedAuthorization {
            binding: GrantBinding {
                provider_id: self.provider.id,
                issuer: identity.issuer,
                client_registration_id: self.provider.registration_id,
                client_id: self.provider.client_id,
                subject: identity.subject,
                mailbox: identity.mailbox,
                grant_id: uuid::Uuid::new_v4().to_string(),
                generation: 1,
                resource: self.provider.resource,
                scopes: self.provider.scopes,
            },
            tokens,
        })
    }
}

#[derive(Clone)]
pub(crate) struct TokenSet {
    pub access_token: SecretValue,
    pub refresh_token: Option<SecretValue>,
    pub expires_at: u64,
    pub target: Option<TokenTarget>,
    pub scopes: Vec<String>,
}
impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenSet([REDACTED])")
    }
}
impl TokenSet {
    pub(crate) fn encode_secret(&self) -> Result<SecretValue, OAuthError> {
        Ok(SecretValue::new(serde_json::to_string(&serde_json::json!({ "access_token": self.access_token.expose_secret(), "refresh_token": self.refresh_token.as_ref().map(SecretValue::expose_secret), "expires_at": self.expires_at, "scopes": self.scopes, "target": self.target })).map_err(|_| OAuthError::Storage)?))
    }
    pub(crate) fn decode_secret(value: &SecretValue) -> Result<Self, OAuthError> {
        if value.expose_secret().len() > 131072 {
            return Err(OAuthError::Storage);
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Stored {
            access_token: SecretValue,
            refresh_token: Option<SecretValue>,
            expires_at: u64,
            scopes: Vec<String>,
            #[serde(default)]
            target: Option<TokenTarget>,
        }
        let value: Stored =
            serde_json::from_str(value.expose_secret()).map_err(|_| OAuthError::Storage)?;
        if value.access_token.expose_secret().is_empty()
            || value.access_token.expose_secret().len() > 16384
            || value
                .access_token
                .expose_secret()
                .chars()
                .any(char::is_control)
            || value.refresh_token.as_ref().is_some_and(|r| {
                r.expose_secret().is_empty()
                    || r.expose_secret().len() > 16384
                    || r.expose_secret().chars().any(char::is_control)
            })
        {
            return Err(OAuthError::Storage);
        }
        Ok(Self {
            access_token: value.access_token,
            refresh_token: value.refresh_token,
            expires_at: value.expires_at,
            scopes: value.scopes,
            target: value.target,
        })
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

type RefreshFuture = futures_util::future::Shared<
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<SecretValue, OAuthError>> + Send>>,
>;
/// One manager per account/registration. The async mutex coalesces refreshes;
/// rotated refresh credentials are persisted before any new access token escapes.
pub(crate) struct TokenManager {
    registration: ProviderRegistration,
    store: Arc<dyn SecretStore>,
    key: String,
    tokens: tokio::sync::Mutex<Option<TokenSet>>,
    cached: std::sync::RwLock<std::collections::HashMap<TokenTarget, TokenSet>>,
    invalidated: Arc<std::sync::atomic::AtomicBool>,
    active_refreshes: std::sync::atomic::AtomicUsize,
    drained: tokio::sync::Notify,
    in_flight: std::sync::Mutex<std::collections::HashMap<TokenTarget, RefreshFuture>>,
    reauthorization: std::sync::atomic::AtomicBool,
    pending_persistence: std::sync::atomic::AtomicBool,
    admission_closed: std::sync::atomic::AtomicBool,
}
impl std::fmt::Debug for TokenManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenManager([REDACTED])")
    }
}
impl TokenManager {
    pub(crate) fn new(
        registration: ProviderRegistration,
        store: Arc<dyn SecretStore>,
        key: String,
    ) -> Result<Self, OAuthError> {
        registration.validate()?;
        Ok(Self {
            registration,
            store,
            key,
            tokens: tokio::sync::Mutex::new(None),
            cached: std::sync::RwLock::new(std::collections::HashMap::new()),
            invalidated: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            active_refreshes: std::sync::atomic::AtomicUsize::new(0),
            drained: tokio::sync::Notify::new(),
            in_flight: std::sync::Mutex::new(std::collections::HashMap::new()),
            reauthorization: std::sync::atomic::AtomicBool::new(false),
            pending_persistence: std::sync::atomic::AtomicBool::new(false),
            admission_closed: std::sync::atomic::AtomicBool::new(false),
        })
    }
    /// Restore data already read through the account journal; performs no I/O.
    pub(crate) fn restore(&mut self, tokens: TokenSet) -> Result<(), OAuthError> {
        if let Some(target) = &tokens.target {
            if !self.registration.permits(target)
                || target.scopes.iter().any(|s| !tokens.scopes.contains(s))
            {
                return Err(OAuthError::InvalidResponse);
            }
            self.cached
                .get_mut()
                .map_err(|_| OAuthError::Storage)?
                .insert(target.clone(), tokens.clone());
        }
        if tokens.refresh_token.is_none() {
            self.reauthorization
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        *self.tokens.get_mut() = Some(tokens);
        Ok(())
    }
    pub(crate) fn needs_reauthorization(&self) -> bool {
        self.reauthorization
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    pub(crate) fn invalidate(&self) -> bool {
        let first = !self
            .invalidated
            .swap(true, std::sync::atomic::Ordering::SeqCst);
        // An invalidated grant never issues another bearer; drop the cached
        // copies so retired managers do not keep them in memory.
        if let Ok(mut cached) = self.cached.write() {
            cached.clear();
        }
        // Owned refresh/persistence work rechecks validity under this lock
        // and never reads the grant after invalidation; if it is busy, that
        // work releases it shortly and the manager is dropped once drained.
        if let Ok(mut tokens) = self.tokens.try_lock() {
            *tokens = None;
        }
        first
    }
    /// A mail server rejected `token` although it has not expired locally
    /// (early revocation, rotation or a shorter server-side lifetime). Forget
    /// it so the next resolution refreshes instead of replaying the bearer.
    /// Only the rejected value is removed; a concurrently refreshed token stays.
    pub(crate) fn reject_access_token(&self, token: &SecretValue) {
        if let Ok(mut cached) = self.cached.write() {
            cached.retain(|_, tokens| tokens.access_token.expose_secret() != token.expose_secret());
        }
    }
    pub(crate) fn is_valid(&self) -> bool {
        !self.invalidated.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Stop new resolutions while allowing already-owned rotation and
    /// persistence work to finish. Serialized with refresh registration.
    pub(crate) fn close_admission(&self) -> Result<(), OAuthError> {
        let _slot = self.in_flight.lock().map_err(|_| OAuthError::Storage)?;
        self.admission_closed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    /// Whether every owned refresh has finished network and secret-store work.
    pub(crate) fn is_drained(&self) -> bool {
        self.active_refreshes
            .load(std::sync::atomic::Ordering::SeqCst)
            == 0
    }
    pub(crate) async fn drain(&self) {
        loop {
            let notified = self.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self
                .active_refreshes
                .load(std::sync::atomic::Ordering::SeqCst)
                == 0
            {
                return;
            }
            notified.await;
        }
    }
    /// Closing the host must not discard a rotation retained after a temporary
    /// keychain failure. Admission is already closed; retry only persistence,
    /// never the token endpoint. Bound retries after explicit write failures; never
    /// abandon an already-running SecretStore write.
    pub(crate) async fn settle_persistence(self: &Arc<Self>) -> Result<(), OAuthError> {
        self.close_admission()?;
        self.drain().await;
        if !self
            .pending_persistence
            .load(std::sync::atomic::Ordering::SeqCst)
            || !self.is_valid()
        {
            return Ok(());
        }
        struct OwnedWrite(Arc<TokenManager>);
        impl Drop for OwnedWrite {
            fn drop(&mut self) {
                self.0
                    .active_refreshes
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                self.0.drained.notify_waiters();
            }
        }
        // Register ownership before spawning: even an unpolled task is counted,
        // and its captured guard retires ownership if that task is dropped.
        self.active_refreshes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let guard = OwnedWrite(self.clone());
        let owner = self.clone();
        tokio::spawn(async move {
            let _owned = guard;
            owner.settle_persistence_owned().await
        })
        .await
        .map_err(|_| OAuthError::Storage)?
    }
    async fn settle_persistence_owned(&self) -> Result<(), OAuthError> {
        for attempt in 0..3 {
            if !self.is_valid()
                || !self
                    .pending_persistence
                    .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(());
            }
            let secret = {
                self.tokens
                    .lock()
                    .await
                    .as_ref()
                    .and_then(|tokens| tokens.encode_secret().ok())
            };
            if let Some(secret) = secret
                && self.persist(secret).await.is_ok()
            {
                self.pending_persistence
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return Ok(());
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(OAuthError::Storage)
    }
    async fn persist(&self, secret: SecretValue) -> Result<(), OAuthError> {
        let store = self.store.clone();
        let key = self.key.clone();
        let invalidated = self.invalidated.clone();
        tokio::task::spawn_blocking(move || {
            if invalidated.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(OAuthError::ReauthenticationRequired);
            }
            store.set(&key, &secret).map_err(|_| OAuthError::Storage)?;
            // Grant keys are unique and never reused for reauthorization.
            if invalidated.load(std::sync::atomic::Ordering::SeqCst) {
                store.delete(&key).map_err(|_| OAuthError::Storage)?;
                return Err(OAuthError::ReauthenticationRequired);
            }
            Ok(())
        })
        .await
        .map_err(|_| OAuthError::Storage)?
    }
    pub(crate) fn cached_token(&self) -> Option<SecretValue> {
        if !self.is_valid() || self.needs_reauthorization() {
            return None;
        }
        self.cached
            .read()
            .ok()?
            .get(&self.registration.default_target())
            .filter(|t| t.expires_at > now().saturating_add(60))
            .map(|t| t.access_token.clone())
    }
    pub(crate) async fn access_token(
        self: &Arc<Self>,
        route: &NetRoute,
    ) -> Result<SecretValue, OAuthError> {
        self.access_token_for(self.registration.default_target(), route)
            .await
    }
    pub(crate) async fn access_token_for(
        self: &Arc<Self>,
        target: TokenTarget,
        route: &NetRoute,
    ) -> Result<SecretValue, OAuthError> {
        let target = TokenTarget::new(target.resource, target.scopes);
        if !self.registration.permits(&target) {
            return Err(OAuthError::InvalidRegistration);
        }
        if !self.is_valid() || self.needs_reauthorization() {
            return Err(OAuthError::ReauthenticationRequired);
        }
        use futures_util::FutureExt;
        let future = {
            let mut slot = self.in_flight.lock().map_err(|_| OAuthError::Storage)?;
            if self
                .admission_closed
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(OAuthError::Network);
            }
            if target == self.registration.default_target()
                && let Some(token) = self.cached_token()
            {
                return Ok(token);
            }
            if let Some(future) = slot.get(&target) {
                future.clone()
            } else {
                let owned = Arc::clone(self);
                let route = NetRoute {
                    proxy: route.proxy.as_ref().map(|p| crate::net::ProxyRoute {
                        kind: p.kind,
                        host: p.host.clone(),
                        port: p.port,
                        auth: p.auth.clone(),
                    }),
                    image_proxy_dns: route.image_proxy_dns,
                };
                let requested_target = target.clone();
                let future: RefreshFuture =
                    (Box::pin(async move { owned.refresh_owned(&route, requested_target).await })
                        as std::pin::Pin<
                            Box<
                                dyn std::future::Future<Output = Result<SecretValue, OAuthError>>
                                    + Send,
                            >,
                        >)
                        .shared();
                slot.insert(target.clone(), future.clone());
                self.active_refreshes
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let task_future = future.clone();
                let owner = Arc::clone(self);
                let completed_target = target.clone();
                tokio::spawn(async move {
                    let result = task_future.await;
                    if matches!(
                        result,
                        Err(OAuthError::ReauthenticationRequired | OAuthError::AuthorizationDenied)
                    ) {
                        owner
                            .reauthorization
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    if let Ok(mut slot) = owner.in_flight.lock() {
                        slot.remove(&completed_target);
                    }
                    owner
                        .active_refreshes
                        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    owner.drained.notify_waiters();
                });
                future
            }
        };
        let result = future.await;
        if matches!(
            result,
            Err(OAuthError::ReauthenticationRequired | OAuthError::AuthorizationDenied)
        ) {
            self.reauthorization
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if !self.is_valid() && !matches!(result, Err(OAuthError::Storage)) {
            return Err(OAuthError::ReauthenticationRequired);
        }
        result
    }
    async fn refresh_owned(
        &self,
        route: &NetRoute,
        target: TokenTarget,
    ) -> Result<SecretValue, OAuthError> {
        let mut guard = self.tokens.lock().await;
        if !self.is_valid() || self.needs_reauthorization() {
            return Err(OAuthError::ReauthenticationRequired);
        }
        if self
            .pending_persistence
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            let tokens = guard.as_ref().ok_or(OAuthError::Storage)?;
            self.persist(tokens.encode_secret()?).await?;
            self.pending_persistence
                .store(false, std::sync::atomic::Ordering::SeqCst);
            if let Some(token_target) = &tokens.target {
                self.cached
                    .write()
                    .map_err(|_| OAuthError::Storage)?
                    .insert(token_target.clone(), tokens.clone());
            }
        }
        if let Some(tokens) = self
            .cached
            .read()
            .map_err(|_| OAuthError::Storage)?
            .get(&target)
            .filter(|t| t.expires_at > now().saturating_add(60))
        {
            return Ok(tokens.access_token.clone());
        }
        let refresh = match guard.as_ref().and_then(|t| t.refresh_token.clone()) {
            Some(value) => value,
            None => {
                let store = self.store.clone();
                let key = self.key.clone();
                let stored = tokio::task::spawn_blocking(move || store.get(&key))
                    .await
                    .map_err(|_| OAuthError::Storage)?
                    .map_err(|_| OAuthError::Storage)?
                    .ok_or(OAuthError::ReauthenticationRequired)?;
                let tokens = TokenSet::decode_secret(&stored)?;
                let refresh = tokens
                    .refresh_token
                    .clone()
                    .ok_or(OAuthError::ReauthenticationRequired)?;
                *guard = Some(tokens);
                refresh
            }
        };
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            if let Some(resource) = &target.resource {
                form.append_pair("resource", resource);
            }
            form.append_pair("scope", &target.scopes.join(" "));
            form.extend_pairs([
                ("grant_type", "refresh_token"),
                ("client_id", &self.registration.client_id),
                ("refresh_token", refresh.expose_secret()),
            ])
            .finish()
        };
        let tokens = match transport::exchange(
            route,
            &self.registration.token_endpoint,
            body,
            Some(refresh),
            &target.scopes,
        )
        .await
        {
            Ok(response) => response.tokens,
            Err(OAuthError::ReauthenticationRequired | OAuthError::AuthorizationDenied) => {
                self.reauthorization
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(tokens) = guard.as_mut() {
                    tokens.refresh_token = None;
                    tokens.expires_at = 0;
                    self.persist(tokens.encode_secret()?).await?;
                }
                self.cached
                    .write()
                    .map_err(|_| OAuthError::Storage)?
                    .clear();
                return Err(OAuthError::ReauthenticationRequired);
            }
            Err(error) => return Err(error),
        };
        // Keep an unpublished rotated token in memory if the keychain is
        // temporarily unavailable. A retry persists this exact rotation before
        // requesting another refresh or returning its access token.
        *guard = Some(tokens.clone());
        self.pending_persistence
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.persist(tokens.encode_secret()?).await?;
        self.pending_persistence
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let access = tokens.access_token.clone();
        self.cached
            .write()
            .map_err(|_| OAuthError::Storage)?
            .insert(target, tokens.clone());
        *guard = Some(tokens);
        Ok(access)
    }
}

#[cfg(test)]
mod tests;
