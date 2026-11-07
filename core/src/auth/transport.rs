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

pub(super) async fn exchange(
    route: &NetRoute,
    endpoint: &Url,
    body: String,
    previous: Option<SecretValue>,
    required_scopes: &[String],
) -> Result<TokenSet, OAuthError> {
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
) -> Result<TokenSet, OAuthError> {
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
        return Err(
            if matches!(
                error.error(),
                oauth2::basic::BasicErrorResponseType::InvalidGrant
            ) {
                OAuthError::ReauthenticationRequired
            } else {
                OAuthError::InvalidResponse
            },
        );
    }
    use oauth2::TokenResponse;
    let parsed: oauth2::basic::BasicTokenResponse =
        serde_json::from_slice(&bytes).map_err(|_| OAuthError::InvalidResponse)?;
    if parsed.token_type() != &oauth2::basic::BasicTokenType::Bearer {
        return Err(OAuthError::InvalidResponse);
    }
    let scopes: Vec<String> = parsed
        .scopes()
        .map(|s| s.iter().map(|s| s.as_str().to_owned()).collect())
        .unwrap_or_else(|| required_scopes.to_vec());
    if required_scopes.iter().any(|scope| !scopes.contains(scope)) {
        return Err(OAuthError::InvalidResponse);
    }
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
    let lifetime = parsed
        .expires_in()
        .map(|d| d.as_secs())
        .filter(|n| *n > 0 && *n <= 31_536_000)
        .ok_or(OAuthError::InvalidResponse)?;
    Ok(TokenSet {
        access_token: SecretValue::new(access.clone()),
        refresh_token: refresh,
        expires_at: now().saturating_add(lifetime),
        scopes,
        target: Some(TokenTarget::new(resource, required_scopes.to_vec())),
    })
}

fn network_error(error: crate::net::NetError) -> OAuthError {
    match error {
        crate::net::NetError::Proxy(_) => OAuthError::Proxy,
        crate::net::NetError::Tls(_) => OAuthError::Tls,
        crate::net::NetError::Timeout => OAuthError::Timeout,
        _ => OAuthError::Network,
    }
}
