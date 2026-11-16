//! POX Autodiscover returns candidates, not credential authorization. An
//! authenticated call requires an exact caller-approved target; neither HTTP
//! nor XML redirects are followed, even when the new host shares a suffix.
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, header};
use hyper_util::rt::TokioIo;

use super::{Endpoint, EwsError, http, soap};
use crate::net::{self, NetRoute};
use crate::secrets::SecretValue;

const MAX_DISCOVERY: usize = 256 * 1024;
const REQUEST_NS: &str =
    "http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006";
const RESPONSE_NS: &str =
    "http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a";
const AUTODISCOVER_NS: &str =
    "http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiscoveryAuth {
    Ntlm,
    Basic,
    Negotiate,
    Other,
    None,
}

pub(crate) struct Discovery {
    pub endpoint: Option<String>,
    pub login: Option<String>,
    pub auth: DiscoveryAuth,
    /// The exact URL contacted, including path, so the host can bind its grant.
    pub source: String,
    pub authentication_required: bool,
    pub redirect: Option<String>,
}

pub(crate) async fn discover(
    route: &NetRoute,
    email: &str,
    target: Option<&str>,
    credentials: Option<(&str, &SecretValue)>,
) -> Result<Discovery, EwsError> {
    // The host owns grant issuance/consumption; this guard prevents accidental
    // addition of credentials to the automatic domain-derived search.
    if credentials.is_some() && target.is_none() {
        return Err(EwsError::Protocol);
    }
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"{REQUEST_NS}\"><Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>{RESPONSE_NS}</AcceptableResponseSchema></Request></Autodiscover>",
        soap::escape(email)
    );
    if let Some(target) = target {
        let endpoint = Endpoint::parse(target)?;
        return tokio::time::timeout(
            Duration::from_secs(30),
            attempt(route, &endpoint, &body, credentials),
        )
        .await
        .map_err(|_| EwsError::Timeout)?;
    }
    let (_, domain) = email.rsplit_once('@').ok_or(EwsError::Protocol)?;
    if domain.is_empty()
        || domain.contains('@')
        || !domain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(EwsError::Protocol);
    }
    // Preserve useful results: optional later candidates cannot erase one by
    // exhausting the shared budget. Every candidate is anonymous.
    let lookup = async {
        let mut last = EwsError::NotFound;
        for host in [format!("autodiscover.{domain}"), domain.to_owned()] {
            let endpoint =
                Endpoint::parse(&format!("https://{host}/autodiscover/autodiscover.xml"))?;
            match tokio::time::timeout(
                Duration::from_secs(8),
                attempt(route, &endpoint, &body, None),
            )
            .await
            {
                Ok(Ok(found)) => return Ok(found),
                Ok(Err(error)) => last = error,
                Err(_) => last = EwsError::Timeout,
            }
        }
        Err(last)
    };
    tokio::time::timeout(Duration::from_secs(20), lookup)
        .await
        .map_err(|_| EwsError::Timeout)?
}

async fn attempt(
    route: &NetRoute,
    endpoint: &Endpoint,
    body: &str,
    credentials: Option<(&str, &SecretValue)>,
) -> Result<Discovery, EwsError> {
    let stream = net::connect(route, endpoint.host(), endpoint.port())
        .await
        .map_err(EwsError::Net)?;
    let (stream, certificate) = net::upgrade_tls_with_peer_certificate(stream, endpoint.host())
        .await
        .map_err(EwsError::Net)?;
    let reply = if let Some((username, password)) = credentials {
        // First learn the scheme on this connection without disclosing secrets.
        let mut connection = http::Connection::with_stream(stream, endpoint).await?;
        let first = connection
            .exchange("Autodiscover", body.to_owned(), None, MAX_DISCOVERY)
            .await?;
        if (300..400).contains(&first.status) {
            return redirect(first, endpoint);
        }
        if first.status == 200 {
            return parse(&first.body, endpoint);
        }
        if first.status != 401 {
            return Err(EwsError::Authentication);
        }
        match offered(&first.headers) {
            DiscoveryAuth::Ntlm | DiscoveryAuth::Negotiate => {
                let binding = http::channel_binding(&certificate)?;
                connection
                    .authenticate_request(
                        "Autodiscover",
                        body.to_owned(),
                        username,
                        password,
                        &binding,
                    )
                    .await?
            }
            // Basic is limited to this exact explicitly approved HTTPS URL;
            // open a fresh tunnel, never an automatic redirect/retry.
            _ if has_basic(&first.headers) => {
                drop(connection);
                basic_request(route, endpoint, body, username, password).await?
            }
            _ => return Err(EwsError::AuthUnsupported),
        }
    } else {
        let mut connection = http::Connection::with_stream(stream, endpoint).await?;
        connection
            .exchange("Autodiscover", body.to_owned(), None, MAX_DISCOVERY)
            .await?
    };
    if (300..400).contains(&reply.status) {
        return redirect(reply, endpoint);
    }
    if reply.status == 401 {
        return Ok(Discovery {
            endpoint: None,
            login: None,
            auth: offered(&reply.headers),
            source: endpoint.as_str().into(),
            authentication_required: true,
            redirect: None,
        });
    }
    if reply.status != 200 {
        return Err(if reply.status == 404 {
            EwsError::NotFound
        } else {
            EwsError::Protocol
        });
    }
    parse(&reply.body, endpoint)
}

async fn basic_request(
    route: &NetRoute,
    endpoint: &Endpoint,
    body: &str,
    username: &str,
    password: &SecretValue,
) -> Result<http::Reply, EwsError> {
    let stream = net::connect(route, endpoint.host(), endpoint.port())
        .await
        .map_err(EwsError::Net)?;
    let stream = net::upgrade_tls(stream, endpoint.host())
        .await
        .map_err(EwsError::Net)?;
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_headers(64)
        .max_buf_size(32 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| EwsError::Protocol)?;
    struct Task(tokio::task::JoinHandle<()>);
    impl Drop for Task {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _task = Task(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let joined = zeroize::Zeroizing::new(format!("{username}:{}", password.expose_secret()));
    let authorization =
        zeroize::Zeroizing::new(format!("Basic {}", STANDARD.encode(joined.as_bytes())));
    let mut authorization_header =
        hyper::header::HeaderValue::from_str(&authorization).map_err(|_| EwsError::Protocol)?;
    authorization_header.set_sensitive(true);
    let authority = &endpoint.url[url::Position::BeforeHost..url::Position::AfterPort];
    let request = Request::builder()
        .method("POST")
        .uri(endpoint.url.path())
        .header(header::HOST, authority)
        .header(header::CONTENT_TYPE, "text/xml; charset=utf-8")
        .header(header::AUTHORIZATION, authorization_header)
        .body(Full::new(Bytes::from(body.to_owned())))
        .map_err(|_| EwsError::Protocol)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| EwsError::Protocol)?;
    http::collect(response, MAX_DISCOVERY).await
}

fn redirect(reply: http::Reply, source: &Endpoint) -> Result<Discovery, EwsError> {
    let next = reply
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(EwsError::Protocol)?;
    let next = source.url.join(next).map_err(|_| EwsError::Protocol)?;
    let next = Endpoint::parse(next.as_str())?;
    Ok(Discovery {
        endpoint: None,
        login: None,
        auth: DiscoveryAuth::None,
        source: source.as_str().into(),
        authentication_required: false,
        redirect: Some(next.as_str().into()),
    })
}

fn has_basic(headers: &hyper::HeaderMap) -> bool {
    headers
        .get_all(header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| {
            value
                .split_whitespace()
                .next()
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("Basic"))
        })
}

fn offered(headers: &hyper::HeaderMap) -> DiscoveryAuth {
    let mut result = DiscoveryAuth::None;
    for value in headers
        .get_all(header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
    {
        for challenge in value.split(',') {
            match challenge
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "ntlm" => return DiscoveryAuth::Ntlm,
                "basic" => result = DiscoveryAuth::Basic,
                "negotiate" if result != DiscoveryAuth::Basic => result = DiscoveryAuth::Negotiate,
                _ if result == DiscoveryAuth::None => result = DiscoveryAuth::Other,
                _ => {}
            }
        }
    }
    result
}

fn parse(bytes: &[u8], source: &Endpoint) -> Result<Discovery, EwsError> {
    if bytes.len() > MAX_DISCOVERY {
        return Err(EwsError::TooLarge);
    }
    let doc = soap::document(bytes)?;
    let root = doc.root_element();
    if !root.has_tag_name((AUTODISCOVER_NS, "Autodiscover"))
        && !root.has_tag_name((RESPONSE_NS, "Autodiscover"))
    {
        return Err(EwsError::Protocol);
    }
    let response = root
        .children()
        .find(|node| node.has_tag_name((RESPONSE_NS, "Response")))
        .ok_or(EwsError::Protocol)?;
    let account = response
        .children()
        .find(|node| node.has_tag_name((RESPONSE_NS, "Account")))
        .ok_or(EwsError::NotFound)?;
    let value = |node: roxmltree::Node<'_, '_>, name: &str| {
        node.children()
            .find(|node| node.has_tag_name((RESPONSE_NS, name)))
            .and_then(|node| node.text())
            .map(str::to_owned)
    };
    if value(account, "Action").as_deref() == Some("redirectUrl") {
        let target = value(account, "RedirectUrl").ok_or(EwsError::Protocol)?;
        let target = Endpoint::parse(&target)?;
        return Ok(Discovery {
            endpoint: None,
            login: None,
            auth: DiscoveryAuth::None,
            source: source.as_str().into(),
            authentication_required: false,
            redirect: Some(target.as_str().into()),
        });
    }
    if value(account, "Action").as_deref() != Some("settings") {
        return Err(EwsError::NotFound);
    }
    // Only an explicit EwsUrl is evidence of EWS; Outlook Anywhere Server/ASUrl
    // are different services and must not be rewritten into guessed endpoints.
    for protocol in account
        .children()
        .filter(|node| node.has_tag_name((RESPONSE_NS, "Protocol")))
    {
        if let Some(url) = value(protocol, "EwsUrl") {
            let endpoint = Endpoint::parse(&url)?;
            let auth = value(protocol, "AuthPackage").unwrap_or_default();
            let auth = if auth.eq_ignore_ascii_case("NTLM") {
                DiscoveryAuth::Ntlm
            } else if auth.eq_ignore_ascii_case("Basic") {
                DiscoveryAuth::Basic
            } else if auth.eq_ignore_ascii_case("Negotiate") {
                DiscoveryAuth::Negotiate
            } else if auth.is_empty() {
                DiscoveryAuth::None
            } else {
                DiscoveryAuth::Other
            };
            return Ok(Discovery {
                endpoint: Some(endpoint.as_str().into()),
                login: value(protocol, "LoginName")
                    .filter(|value| value.len() <= 320 && !value.chars().any(char::is_control)),
                auth,
                source: source.as_str().into(),
                authentication_required: false,
                redirect: None,
            });
        }
    }
    Err(EwsError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_endpoint_is_a_candidate_not_a_credential_grant() {
        let xml = format!(
            "<Autodiscover xmlns=\"{AUTODISCOVER_NS}\"><Response xmlns=\"{RESPONSE_NS}\"><Account><Action>settings</Action><Protocol><EwsUrl>https://other.example/EWS/Exchange.asmx</EwsUrl></Protocol></Account></Response></Autodiscover>"
        );
        let source =
            Endpoint::parse("https://autodiscover.example/autodiscover/autodiscover.xml").unwrap();
        let result = parse(xml.as_bytes(), &source).unwrap();
        assert_eq!(
            result.endpoint.as_deref(),
            Some("https://other.example/EWS/Exchange.asmx")
        );
        assert!(!result.authentication_required);
        assert_eq!(result.source, source.as_str());
    }
    #[test]
    fn negotiate_does_not_claim_kerberos_only_or_ntlm() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(header::WWW_AUTHENTICATE, "Negotiate".parse().unwrap());
        assert_eq!(offered(&headers), DiscoveryAuth::Negotiate);
        headers.append(header::WWW_AUTHENTICATE, "NTLM".parse().unwrap());
        assert_eq!(offered(&headers), DiscoveryAuth::Ntlm);
    }
}
