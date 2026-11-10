use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, header};
use hyper_util::rt::TokioIo;

struct Connection(tokio::task::JoinHandle<()>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// One token endpoint answer. `id_token` is returned to the caller for
/// identity checks and is never stored.
#[derive(Debug)]
pub(super) struct ExchangeResponse {
    pub tokens: TokenSet,
    pub id_token: Option<SecretValue>,
}

pub(super) async fn exchange(
    route: &NetRoute,
    endpoint: &Url,
    body: String,
    previous: Option<SecretValue>,
    required_scopes: &[String],
) -> Result<ExchangeResponse, OAuthError> {
    tokio::time::timeout(
        Duration::from_secs(30),
        exchange_inner(route, endpoint, body, previous, required_scopes),
    )
    .await
    .map_err(|_| OAuthError::Timeout)?
}
async fn exchange_inner(
    route: &NetRoute,
    endpoint: &Url,
    body: String,
    previous: Option<SecretValue>,
    required_scopes: &[String],
) -> Result<ExchangeResponse, OAuthError> {
    let host = endpoint.host_str().ok_or(OAuthError::InvalidRegistration)?;
    let mut stream = crate::net::connect(
        route,
        host,
        endpoint
            .port_or_known_default()
            .ok_or(OAuthError::InvalidRegistration)?,
    )
    .await
    .map_err(network_error)?;
    if endpoint.scheme() == "https" {
        stream = crate::net::upgrade_tls(stream, host)
            .await
            .map_err(network_error)?;
    }
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_buf_size(16384)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| OAuthError::Network)?;
    let _connection = Connection(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let authority = &endpoint[url::Position::BeforeHost..url::Position::AfterPort];
    let resource = url::form_urlencoded::parse(body.as_bytes())
        .find(|(key, _)| key == "resource")
        .map(|(_, v)| v.into_owned());
    let request = Request::post(endpoint.path())
        .header(header::HOST, authority)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ACCEPT, "application/json")
        .body(Full::new(Bytes::from(body)))
        .map_err(|_| OAuthError::InvalidRegistration)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| OAuthError::Network)?;
    let status = response.status();
    if status.as_u16() == 429 {
        return Err(OAuthError::RateLimited);
    }
    if status.is_server_error() {
        return Err(OAuthError::ServerUnavailable);
    }
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| OAuthError::Network)?;
        if let Some(data) = frame.data_ref() {
            if bytes.len() + data.len() > 65536 {
                return Err(OAuthError::InvalidResponse);
            }
            bytes.extend_from_slice(data);
        }
    }
    if !status.is_success() {
        let error: oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType> =
            serde_json::from_slice(&bytes).map_err(|_| OAuthError::InvalidResponse)?;
        return Err(error_kind(error.error()));
    }
    use oauth2::TokenResponse;
    let parsed: oauth2::basic::BasicTokenResponse =
        serde_json::from_slice(&bytes).map_err(|_| OAuthError::InvalidResponse)?;
    let id_token = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("id_token")?.as_str().map(str::to_owned))
        .filter(|t| !t.is_empty() && t.len() <= 16384 && !t.chars().any(char::is_control))
        .map(SecretValue::new);
    if parsed.token_type() != &oauth2::basic::BasicTokenType::Bearer {
        return Err(OAuthError::InvalidResponse);
    }
    let scopes = granted_scopes(
        parsed
            .scopes()
            .map(|s| s.iter().map(|s| s.as_str().to_owned()).collect()),
        required_scopes,
    )?;
    let access = parsed.access_token().secret();
    if access.is_empty() || access.len() > 16384 || access.chars().any(char::is_control) {
        return Err(OAuthError::InvalidResponse);
    }
    let refresh = match parsed.refresh_token() {
        Some(s)
            if s.secret().is_empty()
                || s.secret().len() > 16384
                || s.secret().chars().any(char::is_control) =>
        {
            return Err(OAuthError::InvalidResponse);
        }
        Some(s) => Some(SecretValue::new(s.secret().clone())),
        None => previous,
    };
    // RFC 6749 §5.1 only recommends `expires_in`. Without it, assume a short
    // lifetime; a mail server rejecting the bearer earlier forces a refresh.
    let lifetime = match parsed.expires_in().map(|d| d.as_secs()) {
        None => DEFAULT_TOKEN_LIFETIME_SECS,
        Some(n) if n > 0 && n <= 31_536_000 => n,
        Some(_) => return Err(OAuthError::InvalidResponse),
    };
    Ok(ExchangeResponse {
        tokens: TokenSet {
            access_token: SecretValue::new(access.clone()),
            refresh_token: refresh,
            expires_at: now().saturating_add(lifetime),
            scopes,
            target: Some(TokenTarget::new(resource, required_scopes.to_vec())),
        },
        id_token,
    })
}

/// Lifetime assumed when a token response omits `expires_in`.
const DEFAULT_TOKEN_LIFETIME_SECS: u64 = 30 * 60;

/// Request-level scopes that authorization servers commonly honour without
/// echoing them in the granted `scope` list (for example Microsoft identity
/// omits `offline_access`). They never grant mailbox access by themselves.
const UNECHOED_META_SCOPES: &[&str] = &["offline_access", "openid", "profile", "email"];

/// Validates the granted scope list (RFC 6749 §5.1: omitted means "as
/// requested"). Every requested resource scope must be granted; requested meta
/// scopes missing from the echo are kept so stored grants stay self-consistent.
pub(super) fn granted_scopes(
    granted: Option<Vec<String>>,
    required_scopes: &[String],
) -> Result<Vec<String>, OAuthError> {
    let Some(mut scopes) = granted else {
        return Ok(required_scopes.to_vec());
    };
    for scope in required_scopes {
        if scopes.contains(scope) {
            continue;
        }
        if !UNECHOED_META_SCOPES.contains(&scope.as_str()) {
            return Err(OAuthError::InvalidResponse);
        }
        scopes.push(scope.clone());
    }
    Ok(scopes)
}

fn network_error(error: crate::net::NetError) -> OAuthError {
    match error {
        crate::net::NetError::Proxy(_) => OAuthError::Proxy,
        crate::net::NetError::Tls(_) => OAuthError::Tls,
        crate::net::NetError::Timeout => OAuthError::Timeout,
        _ => OAuthError::Network,
    }
}

/// Maps a token endpoint error to the one case callers act on: the grant can
/// no longer be used silently and the user has to sign in again. Everything
/// else stays an invalid response without echoing server text.
fn error_kind(error: &oauth2::basic::BasicErrorResponseType) -> OAuthError {
    use oauth2::basic::BasicErrorResponseType as Kind;
    match error {
        Kind::InvalidGrant => OAuthError::ReauthenticationRequired,
        Kind::Extension(name)
            if matches!(
                name.as_str(),
                "interaction_required" | "consent_required" | "login_required"
            ) =>
        {
            OAuthError::ReauthenticationRequired
        }
        _ => OAuthError::InvalidResponse,
    }
}
