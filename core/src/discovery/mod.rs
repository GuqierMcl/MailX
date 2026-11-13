//! Exchange / IMAP / SMTP server discovery for the add-account form.
//!
//! This is the only place MailX offers to send mail credentials to a host
//! derived from the mailbox domain instead of one the user typed. A lookup is
//! opt-in per call, HTTPS-only and bounded; it only reports endpoints for the
//! form to fill in and never creates, edits or stores an account. Every
//! request uses the caller's `NetRoute`: a configured proxy is never bypassed
//! by a direct connection. Credentials go only to domain-derived Autodiscover
//! endpoints; the Exchange "HTTP connection" (EWS) fallback probe is anonymous.

// The add-account caller is wired up outside this module; until it lands in
// this crate none of the items below has an in-crate user.
#![allow(dead_code)]

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, header};
use hyper_util::rt::TokioIo;
use url::Url;

use crate::accounts::types::TlsMode;
use crate::net::{self, NetError, NetRoute};
use crate::secrets::SecretValue;

/// One lookup covers both candidates, any redirect hop and the EWS probe.
const BUDGET: Duration = Duration::from_secs(20);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
/// A lookup follows at most this many `redirectUrl` / HTTP redirect hops.
const MAX_HOPS: usize = 2;
const MAX_URL_BYTES: usize = 8192;
const MAX_FIELD_BYTES: usize = 320;
const STANDARD_EWS_PATH: &str = "/EWS/Exchange.asmx";

const REQUEST_SCHEMA: &str =
    "http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006";
const RESPONSE_SCHEMA: &str =
    "http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EwsAuth {
    Ntlm,
    Basic,
}

/// One server MailX could connect to. `username` is the login name the server
/// suggests, which for Exchange commonly is `DOMAIN\user`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveredEndpoint {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub username: Option<String>,
}

/// An Exchange Web Services endpoint reached over HTTPS (the "HTTP connection"
/// in Outlook/Foxmail). `url` is the full https URL of Exchange.asmx.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EwsEndpoint {
    pub url: String,
    pub auth: EwsAuth,
    pub username: Option<String>,
    pub cert_principal_name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Settings {
    pub imap: Option<DiscoveredEndpoint>,
    pub smtp: Option<DiscoveredEndpoint>,
    pub ews: Option<EwsEndpoint>,
    /// Origin that answered, shown to the user so they see where values came from.
    pub source: Option<String>,
}

/// Why a lookup produced no settings. Fixed variants only: server text is
/// untrusted and never reaches the user or an ordinary log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiscoveryError {
    /// The endpoint requires authentication and the request carried none.
    NeedsCredentials,
    /// The endpoint rejected the supplied credentials or reported an account error.
    Rejected,
    NotFound,
    Network,
    Proxy,
    Tls,
    Timeout,
    /// The answer could not be used: not HTTPS, malformed, no TLS endpoint.
    Unsupported,
}

/// Autodiscover authenticates with SMTP-style Basic credentials.
pub(crate) struct Credentials<'a> {
    pub username: &'a str,
    pub password: &'a SecretValue,
}

/// Looks up the settings for `email` through the mailbox domain's
/// Autodiscover endpoints and an anonymous EWS probe, along the route the
/// form selected.
pub(crate) async fn discover(
    route: &NetRoute,
    email: &str,
    credentials: Option<&Credentials<'_>>,
) -> Result<Settings, DiscoveryError> {
    match tokio::time::timeout(BUDGET, look_up(route, email, credentials)).await {
        Ok(result) => result,
        Err(_) => Err(DiscoveryError::Timeout),
    }
}

async fn look_up(
    route: &NetRoute,
    email: &str,
    credentials: Option<&Credentials<'_>>,
) -> Result<Settings, DiscoveryError> {
    let mut domain = email
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
        .ok_or(DiscoveryError::Unsupported)?;
    normalize_host(&mut domain).ok_or(DiscoveryError::Unsupported)?;

    let body = request_body(email);
    let mut settings = Settings::default();
    let mut needs_credentials = false;
    let mut rejected = false;
    let mut transport: Option<DiscoveryError> = None;

    // The service name first, then the bare domain: some deployments only
    // publish one of the two.
    for candidate in [
        format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"),
        format!("https://{domain}/autodiscover/autodiscover.xml"),
    ] {
        match attempt(route, &candidate, &body, credentials, &domain).await {
            Ok(answer) => {
                merge(&mut settings, answer);
                if settings.imap.is_some() && settings.smtp.is_some() && settings.ews.is_some() {
                    return Ok(settings);
                }
            }
            Err(DiscoveryError::NeedsCredentials) => needs_credentials = true,
            Err(DiscoveryError::Rejected) => rejected = true,
            // "Not found" is the weakest reason; a real transport error wins.
            Err(DiscoveryError::NotFound) => {}
            Err(error) => transport = transport.or(Some(error)),
        }
    }

    // On-premise "Exchange over HTTP" often has no working Autodiscover and
    // only offers Negotiate/NTLM on EWS over 443. Probe it anonymously.
    if settings.ews.is_none() {
        for candidate in [
            format!("https://mail.{domain}{STANDARD_EWS_PATH}"),
            format!("https://{domain}{STANDARD_EWS_PATH}"),
        ] {
            match probe_ews(route, &candidate, &domain).await {
                Ok(Some((endpoint, origin))) => {
                    settings.ews = Some(endpoint);
                    settings.source = settings.source.or(Some(origin));
                    break;
                }
                Ok(None) | Err(DiscoveryError::NotFound) => {}
                Err(error) => transport = transport.or(Some(error)),
            }
        }
    }

    if settings.imap.is_some() || settings.smtp.is_some() || settings.ews.is_some() {
        return Ok(settings);
    }
    if needs_credentials {
        return Err(DiscoveryError::NeedsCredentials);
    }
    if rejected {
        return Err(DiscoveryError::Rejected);
    }
    Err(transport.unwrap_or(DiscoveryError::NotFound))
}

fn merge(target: &mut Settings, answer: Settings) {
    if target.imap.is_none() {
        target.imap = answer.imap;
    }
    if target.smtp.is_none() {
        target.smtp = answer.smtp;
    }
    if target.ews.is_none() {
        target.ews = answer.ews;
    }
    if target.source.is_none() {
        target.source = answer.source;
    }
}

async fn attempt(
    route: &NetRoute,
    candidate: &str,
    body: &str,
    credentials: Option<&Credentials<'_>>,
    domain: &str,
) -> Result<Settings, DiscoveryError> {
    let mut url = checked_url(candidate, domain)?;
    for _ in 0..=MAX_HOPS {
        let origin = origin(&url);
        let reply = request(route, &url, "POST", body, credentials, true).await?;
        // Relative redirects resolve against the endpoint we called.
        if matches!(reply.status, 301 | 302 | 303 | 307 | 308) {
            let location = reply.location.ok_or(DiscoveryError::Unsupported)?;
            let next = url
                .join(&location)
                .map_err(|_| DiscoveryError::Unsupported)?;
            url = checked_url(next.as_str(), domain)?;
            continue;
        }
        if matches!(reply.status, 401 | 403) {
            return Err(if credentials.is_some() {
                DiscoveryError::Rejected
            } else {
                DiscoveryError::NeedsCredentials
            });
        }
        if reply.status != 200 {
            return Err(DiscoveryError::NotFound);
        }
        match parse_settings(&reply.body, &origin)? {
            Parsed::Settings(settings) => return Ok(settings),
            Parsed::Redirect(next) => url = checked_url(&next, domain)?,
            Parsed::Rejected => return Err(DiscoveryError::Rejected),
            Parsed::Empty => return Ok(Settings::default()),
        }
    }
    Err(DiscoveryError::Unsupported)
}

/// A credential-free GET that only inspects `WWW-Authenticate`. The response
/// body is never read, so no server text is kept.
async fn probe_ews(
    route: &NetRoute,
    candidate: &str,
    domain: &str,
) -> Result<Option<(EwsEndpoint, String)>, DiscoveryError> {
    let url = checked_url(candidate, domain)?;
    let origin = origin(&url);
    let reply = request(route, &url, "GET", "", None, false).await?;
    let Some(auth) = reply.auth else {
        return Ok(None);
    };
    Ok(Some((
        EwsEndpoint {
            url: candidate.to_owned(),
            auth,
            username: None,
            cert_principal_name: None,
        },
        origin,
    )))
}

/// Whether an absolute URL is safe to contact: HTTPS only, no user info, and
/// a host equal to or a subdomain of the mailbox domain.
fn is_allowed_host(url: &str, domain: &str) -> bool {
    if url.len() > MAX_URL_BYTES {
        return false;
    }
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none()
    {
        return false;
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// Every candidate and redirect must pass this before a request is made.
fn checked_url(value: &str, domain: &str) -> Result<Url, DiscoveryError> {
    if !is_allowed_host(value, domain) {
        return Err(DiscoveryError::Unsupported);
    }
    Url::parse(value).map_err(|_| DiscoveryError::Unsupported)
}

fn origin(url: &Url) -> String {
    format!(
        "https://{}",
        &url[url::Position::BeforeHost..url::Position::AfterPort]
    )
}

struct Reply {
    status: u16,
    location: Option<String>,
    /// Authentication schemes reduced to a fixed variant; raw header text is
    /// never kept.
    auth: Option<EwsAuth>,
    body: String,
}

async fn request(
    route: &NetRoute,
    url: &Url,
    method: &str,
    body: &str,
    credentials: Option<&Credentials<'_>>,
    read_body: bool,
) -> Result<Reply, DiscoveryError> {
    let host = url
        .host_str()
        .ok_or(DiscoveryError::Unsupported)?
        .to_owned();
    let port = url
        .port_or_known_default()
        .ok_or(DiscoveryError::Unsupported)?;
    let exchange = async {
        let stream = net::connect(route, &host, port).await.map_err(net_error)?;
        // HTTPS only: Autodiscover may carry credentials, so it never runs in
        // clear text. The anonymous probe uses the same trusted path.
        let stream = net::upgrade_tls(stream, &host).await.map_err(net_error)?;
        send(stream, url, method, body, credentials, read_body).await
    };
    match tokio::time::timeout(REQUEST_TIMEOUT, exchange).await {
        Ok(result) => result,
        Err(_) => Err(DiscoveryError::Timeout),
    }
}

fn net_error(error: NetError) -> DiscoveryError {
    match error {
        NetError::Connect(_) => DiscoveryError::Network,
        NetError::Proxy(_) => DiscoveryError::Proxy,
        NetError::Tls(_) => DiscoveryError::Tls,
        NetError::Timeout => DiscoveryError::Timeout,
    }
}

struct ConnectionTask(tokio::task::JoinHandle<()>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn send(
    stream: net::BoxedStream,
    url: &Url,
    method: &str,
    body: &str,
    credentials: Option<&Credentials<'_>>,
    read_body: bool,
) -> Result<Reply, DiscoveryError> {
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_headers(64)
        .max_buf_size(32 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| DiscoveryError::Network)?;
    // Dropping the guard aborts the connection task on every early return.
    let _connection = ConnectionTask(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, authority)
        .header(header::USER_AGENT, "MailX")
        .header(header::ACCEPT, "text/xml, application/xml")
        .header(header::CONTENT_TYPE, "text/xml; charset=utf-8");
    if let Some(credentials) = credentials {
        builder = builder.header(
            header::AUTHORIZATION,
            basic_authorization(credentials.username, credentials.password),
        );
    }
    let request = builder
        .body(Full::new(Bytes::from(body.to_owned())))
        .map_err(|_| DiscoveryError::Unsupported)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| DiscoveryError::Network)?;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let auth = auth_from_headers(response.headers());
    let body = if read_body {
        let mut collected = Vec::new();
        let mut incoming = response.into_body();
        while let Some(frame) = incoming.frame().await {
            let frame = frame.map_err(|_| DiscoveryError::Network)?;
            if let Ok(data) = frame.into_data() {
                if collected.len() + data.len() > MAX_RESPONSE_BYTES {
                    return Err(DiscoveryError::Unsupported);
                }
                collected.extend_from_slice(&data);
            }
        }
        String::from_utf8(collected).map_err(|_| DiscoveryError::Unsupported)?
    } else {
        String::new()
    };
    Ok(Reply {
        status,
        location,
        auth,
        body,
    })
}

fn auth_from_headers(headers: &hyper::HeaderMap) -> Option<EwsAuth> {
    let mut ntlm = false;
    let mut basic = false;
    for value in headers.get_all(header::WWW_AUTHENTICATE) {
        let Ok(text) = value.to_str() else {
            continue;
        };
        for part in text.split(',') {
            let scheme = part.split_whitespace().next().unwrap_or_default();
            if scheme.eq_ignore_ascii_case("ntlm") || scheme.eq_ignore_ascii_case("negotiate") {
                ntlm = true;
            } else if scheme.eq_ignore_ascii_case("basic") {
                basic = true;
            }
        }
    }
    if ntlm {
        Some(EwsAuth::Ntlm)
    } else if basic {
        Some(EwsAuth::Basic)
    } else {
        None
    }
}

fn basic_authorization(username: &str, password: &SecretValue) -> String {
    let joined = format!("{username}:{}", password.expose_secret());
    format!("Basic {}", STANDARD.encode(joined))
}

/// The POX request body. Only the address is interpolated, escaped so a
/// mailbox cannot close the element early.
fn request_body(email: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <Autodiscover xmlns=\"{REQUEST_SCHEMA}\">\
         <Request>\
         <EMailAddress>{}</EMailAddress>\
         <AcceptableResponseSchema>{RESPONSE_SCHEMA}</AcceptableResponseSchema>\
         </Request>\
         </Autodiscover>",
        escape(email)
    )
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[derive(Debug)]
enum Parsed {
    Settings(Settings),
    Redirect(String),
    /// The server answered with an `<Error>` element or an unusable account.
    Rejected,
    /// Well-formed but without a settings answer for this mailbox.
    Empty,
}

/// Reads the POX answer without a general XML parser: the response is
/// untrusted, so only a bounded subset of element names is honoured, DTDs are
/// refused, and every extracted value is validated before it is offered to
/// the form.
fn parse_settings(xml: &str, source: &str) -> Result<Parsed, DiscoveryError> {
    if xml.len() > MAX_RESPONSE_BYTES || has_dtd(xml) {
        return Err(DiscoveryError::Unsupported);
    }
    // Namespace prefixes are removed up front so all scanners below are
    // prefix-agnostic without a full XML parser.
    let normalized = strip_prefixes(xml);
    let xml = normalized.as_str();

    if let Some(action) = element(xml, "Action")
        && action.eq_ignore_ascii_case("redirectUrl")
    {
        // POX redirect answers normally use RedirectAddr; older servers use
        // RedirectUrl. Both are re-validated by the caller before a request.
        let target = element(xml, "RedirectAddr")
            .or_else(|| element(xml, "RedirectUrl"))
            .ok_or(DiscoveryError::Unsupported)?;
        return Ok(Parsed::Redirect(target));
    }
    if let Some(action) = element(xml, "Action")
        && action.eq_ignore_ascii_case("error")
    {
        return Ok(Parsed::Rejected);
    }
    if element(xml, "Error").is_some() || element(xml, "ErrorCode").is_some() {
        return Ok(Parsed::Rejected);
    }

    // Settings for other account types may share the document; only the first
    // email account block is read.
    let account = blocks(xml, "Account")
        .into_iter()
        .next()
        .map_or(xml, |block| block.inner);

    let mut settings = Settings {
        source: Some(source.to_owned()),
        ..Settings::default()
    };
    for block in blocks(account, "Protocol") {
        let kind = element(block.inner, "Type")
            .or_else(|| attribute(block.open, "Type").map(str::to_owned));
        let Some(kind) = kind else {
            continue;
        };
        let kind = kind.trim();
        if kind.eq_ignore_ascii_case("IMAP") {
            if settings.imap.is_none() {
                settings.imap = endpoint(block.inner);
            }
        } else if kind.eq_ignore_ascii_case("SMTP") {
            if settings.smtp.is_none() {
                settings.smtp = endpoint(block.inner);
            }
        } else if is_ews_type(kind) && settings.ews.is_none() {
            settings.ews = ews_endpoint(block.inner);
        }
    }
    if settings.imap.is_none() && settings.smtp.is_none() && settings.ews.is_none() {
        return Ok(Parsed::Empty);
    }
    Ok(Parsed::Settings(settings))
}

fn is_ews_type(kind: &str) -> bool {
    ["EXCH", "EXPR", "EXHTTP", "HTTP", "WEB"]
        .iter()
        .any(|candidate| kind.eq_ignore_ascii_case(candidate))
}

fn endpoint(block: &str) -> Option<DiscoveredEndpoint> {
    let mut host = element(block, "Server")?;
    normalize_host(&mut host)?;
    let port: u16 = element(block, "Port")?.parse().ok()?;
    if port == 0 {
        return None;
    }
    let ssl = element(block, "SSL");
    let encryption = element(block, "Encryption");
    let tls = tls_mode(port, ssl.as_deref(), encryption.as_deref())?;
    // <SPA> and <AuthRequired> describe transport authentication. The wire
    // contract has no auth field: MailX's IMAP/SMTP layers negotiate SPA
    // themselves, so reading them here only confirms this is a settings block
    // and never changes the endpoint we report.
    let _ = (element(block, "SPA"), element(block, "AuthRequired"));
    let username = clean_text(element(block, "LoginName"));
    Some(DiscoveredEndpoint {
        host,
        port,
        tls,
        username,
    })
}

/// `<SSL>on</SSL>` and `<Encryption>TLS</Encryption>` overlap in practice; an
/// explicit `Encryption` wins. Otherwise SSL and the well-known port are
/// combined: MailX only speaks TLS, so a clear-text endpoint is skipped.
fn tls_mode(port: u16, ssl: Option<&str>, encryption: Option<&str>) -> Option<TlsMode> {
    match encryption.map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("TLS") => return Some(TlsMode::StartTls),
        Some(value) if value.eq_ignore_ascii_case("SSL") => return Some(TlsMode::Implicit),
        Some(value) if value.eq_ignore_ascii_case("None") => return None,
        _ => {}
    }
    let by_port = match port {
        993 | 465 => Some(TlsMode::Implicit),
        143 | 25 | 587 => Some(TlsMode::StartTls),
        _ => None,
    };
    match ssl.map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("on") => Some(TlsMode::Implicit),
        Some(value) if value.eq_ignore_ascii_case("off") => by_port.or(Some(TlsMode::StartTls)),
        _ => by_port,
    }
}

fn ews_endpoint(block: &str) -> Option<EwsEndpoint> {
    // Never offer a clear-text EWS endpoint: the "HTTP connection" in
    // Outlook/Foxmail still runs over HTTPS.
    if element(block, "SSL").is_some_and(|value| value.trim().eq_ignore_ascii_case("off")) {
        return None;
    }
    let url = [
        "EwsUrl",
        "ASUrl",
        "OOFUrl",
        "Server",
        "ServerExclusiveConnect",
    ]
    .into_iter()
    .find_map(|tag| element(block, tag).and_then(|value| ews_url_from(&value)))?;
    let auth = ews_auth_from(
        element(block, "AuthPackage").as_deref(),
        element(block, "AuthRequired").as_deref(),
    );
    Some(EwsEndpoint {
        url,
        auth,
        username: clean_text(element(block, "LoginName")),
        cert_principal_name: clean_text(element(block, "CertPrinName")),
    })
}

/// Builds the https URL of Exchange.asmx from a value that may be a full URL,
/// a bare host, or a `GUID@domain` Outlook-Anywhere server name. An existing
/// `.asmx` path is kept; anything else gets the standard path. Plain http and
/// other schemes are refused rather than downgraded.
fn ews_url_from(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_URL_BYTES || value.chars().any(char::is_control) {
        return None;
    }
    // Outlook Anywhere <Server> is often `GUID@domain`: take the domain and
    // fall back to the conventional `mail.{domain}` host.
    if let Some((_, domain)) = value.rsplit_once('@') {
        let mut host = format!("mail.{}", domain.trim());
        normalize_host(&mut host)?;
        return Some(format!("https://{host}{STANDARD_EWS_PATH}"));
    }
    let rest = if let Some(rest) = value.strip_prefix("https://") {
        rest
    } else if value.contains("://") || value.starts_with("http:") {
        return None;
    } else {
        value
    };
    // Queries and fragments are dropped: they are server-controlled free text.
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, String::new()),
    };
    let authority = normalize_authority(authority)?;
    let path = if path.to_ascii_lowercase().ends_with(".asmx") {
        path
    } else {
        STANDARD_EWS_PATH.to_owned()
    };
    Some(format!("https://{authority}{path}"))
}

fn ews_auth_from(auth_package: Option<&str>, auth_required: Option<&str>) -> EwsAuth {
    let offered = format!(
        "{} {}",
        auth_package.unwrap_or_default(),
        auth_required.unwrap_or_default()
    )
    .to_ascii_lowercase();
    if offered.contains("ntlm") || offered.contains("negotiate") {
        EwsAuth::Ntlm
    } else if offered.contains("basic") {
        EwsAuth::Basic
    } else {
        // No explicit scheme: these Outlook-Anywhere / EWS types default to
        // NTLM, which is what on-premise "Exchange over HTTP" deployments use.
        EwsAuth::Ntlm
    }
}

fn clean_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let text = text.trim();
        (!text.is_empty() && text.len() <= MAX_FIELD_BYTES && !text.chars().any(char::is_control))
            .then(|| text.to_owned())
    })
}

/// Trims and lower-cases a host name, rejecting paths, bad labels and IPs.
fn normalize_host(host: &mut String) -> Option<()> {
    *host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok()
        || host.is_empty()
        || host.len() > 253
        || !host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return None;
    }
    Some(())
}

/// An authority without user info, with an optional explicit port.
fn normalize_authority(authority: &str) -> Option<String> {
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            (host, Some(port))
        }
        _ => (authority, None),
    };
    let mut host = host.to_owned();
    normalize_host(&mut host)?;
    match port {
        Some(port) => {
            let port: u16 = port.parse().ok()?;
            (port != 0).then(|| format!("{host}:{port}"))
        }
        None => Some(host),
    }
}

fn has_dtd(xml: &str) -> bool {
    let lower = xml.to_ascii_lowercase();
    lower.contains("<!doctype") || lower.contains("<!entity")
}

/// Removes namespace prefixes from element names so the bounded scanners are
/// prefix-agnostic. Only the name right after `<` / `</` is touched;
/// attribute values (URLs, `msstd:` names) are left intact.
fn strip_prefixes(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end_rel) = rest[start + 1..].find('>') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let end = start + 1 + end_rel;
        let tag = &rest[start + 1..end];
        if tag.starts_with('?') || tag.starts_with('!') {
            out.push_str(&rest[start..=end]);
        } else {
            let (slash, name_and_attrs) = tag
                .strip_prefix('/')
                .map_or(("", tag), |stripped| ("/", stripped));
            let name_end = name_and_attrs
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(name_and_attrs.len());
            let name = &name_and_attrs[..name_end];
            let local = name.rsplit_once(':').map_or(name, |(_, local)| local);
            out.push('<');
            out.push_str(slash);
            out.push_str(local);
            out.push_str(&name_and_attrs[name_end..]);
            out.push('>');
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A bounded element: the opening tag (attributes included) and its inner
/// text. A self-closing tag produces an empty block.
struct Block<'a> {
    open: &'a str,
    inner: &'a str,
}

/// Inner text of each `<tag>`…`</tag>` block. Blocks are bounded by the
/// returned slices, never re-scanned.
fn blocks<'a>(fragment: &'a str, tag: &str) -> Vec<Block<'a>> {
    let mut found = Vec::new();
    let mut rest = fragment;
    let close = format!("</{tag}>");
    let open = format!("<{tag}");
    while let Some(start) = rest.find(&open) {
        let after_name = start + open.len();
        // `<Protocol>`, `<Protocol attr="x">` and `<Protocol/>` only;
        // `<Protocols>` is a different element.
        let Some(next) = rest[after_name..].chars().next() else {
            break;
        };
        if !(next == '>' || next == '/' || next.is_whitespace()) {
            rest = &rest[after_name..];
            continue;
        }
        let Some(open_end) = rest[start..].find('>') else {
            break;
        };
        let open_tag = &rest[start + 1..start + open_end];
        if open_tag.trim_end().ends_with('/') {
            found.push(Block {
                open: open_tag.trim_end().trim_end_matches('/').trim_end(),
                inner: "",
            });
            rest = &rest[start + open_end + 1..];
            continue;
        }
        let inner_start = start + open_end + 1;
        let Some(close_offset) = rest[inner_start..].find(&close) else {
            break;
        };
        found.push(Block {
            open: open_tag,
            inner: &rest[inner_start..inner_start + close_offset],
        });
        rest = &rest[inner_start + close_offset + close.len()..];
    }
    found
}

/// Text of the first non-empty `<tag>…</tag>` pair, entity-decoded and
/// trimmed. Self-closing tags contribute no value.
fn element(fragment: &str, tag: &str) -> Option<String> {
    let value = blocks(fragment, tag).into_iter().find_map(|block| {
        let value = block.inner.trim();
        (!value.is_empty()).then_some(value)
    })?;
    Some(decode_entities(value))
}

/// Expands only the five XML predefined entities. DTD-defined and unknown
/// entities are left as literal text, so no server-controlled expansion is
/// possible (DOCTYPE/ENTITY declarations are rejected before parsing).
fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(semi) = after.find(';') else {
            out.push('&');
            rest = after;
            continue;
        };
        let entity = &after[..semi];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            other => {
                out.push('&');
                out.push_str(other);
                out.push(';');
            }
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    out
}

/// Value of `name="…"` in an opening tag (the tag name itself excluded).
fn attribute<'a>(open: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let at = open.find(&needle)?;
    if at == 0 || !open.as_bytes()[at - 1].is_ascii_whitespace() {
        return None;
    }
    let value = &open[at + needle.len()..];
    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = &value[1..];
    let end = value.find(quote)?;
    let value = &value[..end];
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(account: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
             <Autodiscover xmlns=\"{RESPONSE_SCHEMA}\"><Response>\
             <Account><AccountType>email</AccountType>{account}</Account>\
             </Response></Autodiscover>"
        )
    }

    fn settings_response(protocols: &str) -> String {
        response(&format!("<Action>settings</Action>{protocols}"))
    }

    fn settings_of(xml: &str) -> Settings {
        let Parsed::Settings(settings) =
            parse_settings(xml, "https://autodiscover.example.com").unwrap()
        else {
            panic!("expected settings");
        };
        settings
    }

    #[test]
    fn request_names_the_mailbox_and_the_response_schema() {
        let body = request_body("alice@example.com");
        assert!(body.contains(&format!("xmlns=\"{REQUEST_SCHEMA}\"")));
        assert!(body.contains("<EMailAddress>alice@example.com</EMailAddress>"));
        assert!(body.contains(&format!(
            "<AcceptableResponseSchema>{RESPONSE_SCHEMA}</AcceptableResponseSchema>"
        )));
        assert!(request_body("a<b&c@example.com").contains("a&lt;b&amp;c@example.com"));
    }

    #[test]
    fn exch_and_expr_blocks_yield_ntlm_ssl_and_cert_principal() {
        let xml = settings_response(
            "<Protocol><Type>EXCH</Type><Server>mail.example.com</Server><SSL>On</SSL>\
             <AuthPackage>Ntlm</AuthPackage><CertPrinName>msstd:mail.example.com</CertPrinName>\
             <LoginName>DOMAIN\\alice</LoginName></Protocol>\
             <Protocol><Type>EXPR</Type><Server>mail.example.com</Server><SSL>On</SSL>\
             <AuthPackage>Ntlm, Negotiate</AuthPackage>\
             <CertPrinName>msstd:mail.example.com</CertPrinName></Protocol>",
        );
        let settings = settings_of(&xml);
        assert_eq!(
            settings.ews,
            Some(EwsEndpoint {
                url: "https://mail.example.com/EWS/Exchange.asmx".into(),
                auth: EwsAuth::Ntlm,
                username: Some("DOMAIN\\alice".into()),
                cert_principal_name: Some("msstd:mail.example.com".into()),
            })
        );
        assert_eq!(
            settings.source.as_deref(),
            Some("https://autodiscover.example.com")
        );
    }

    #[test]
    fn ews_url_is_used_directly_and_other_paths_get_the_standard_one() {
        let direct = settings_response(
            "<Protocol><Type>EXHTTP</Type>\
             <EwsUrl>https://mail.example.com/EWS/Exchange.asmx</EwsUrl>\
             <SSL>On</SSL><AuthPackage>Basic</AuthPackage></Protocol>",
        );
        let endpoint = settings_of(&direct).ews.unwrap();
        assert_eq!(endpoint.url, "https://mail.example.com/EWS/Exchange.asmx");
        assert_eq!(endpoint.auth, EwsAuth::Basic);

        let other_path = settings_response(
            "<Protocol><Type>EXHTTP</Type>\
             <EwsUrl>https://mail.example.com/owa/service.asmx</EwsUrl>\
             <SSL>On</SSL></Protocol>",
        );
        assert_eq!(
            settings_of(&other_path).ews.unwrap().url,
            "https://mail.example.com/owa/service.asmx"
        );

        let not_asmx = settings_response(
            "<Protocol><Type>EXHTTP</Type>\
             <EwsUrl>https://mail.example.com/owa/</EwsUrl><SSL>On</SSL></Protocol>",
        );
        assert_eq!(
            settings_of(&not_asmx).ews.unwrap().url,
            "https://mail.example.com/EWS/Exchange.asmx"
        );
    }

    #[test]
    fn guid_at_domain_server_falls_back_to_mail_host() {
        let xml = settings_response(
            "<Protocol><Type>EXPR</Type>\
             <Server>11111111-2222-3333-4444-555555555555@contoso.com</Server>\
             <SSL>On</SSL><AuthPackage>Ntlm</AuthPackage></Protocol>",
        );
        assert_eq!(
            settings_of(&xml).ews.unwrap().url,
            "https://mail.contoso.com/EWS/Exchange.asmx"
        );
        assert_eq!(
            ews_url_from("11111111-2222-3333-4444-555555555555@contoso.com"),
            Some("https://mail.contoso.com/EWS/Exchange.asmx".to_owned())
        );
    }

    #[test]
    fn server_exclusive_connect_is_used_when_no_other_url() {
        let xml = settings_response(
            "<Protocol><Type>EXHTTP</Type>\
             <ServerExclusiveConnect>mail.example.com</ServerExclusiveConnect>\
             <SSL>On</SSL></Protocol>",
        );
        let endpoint = settings_of(&xml).ews.unwrap();
        assert_eq!(endpoint.url, "https://mail.example.com/EWS/Exchange.asmx");
        // Without an explicit AuthPackage these protocol types default to NTLM.
        assert_eq!(endpoint.auth, EwsAuth::Ntlm);
    }

    #[test]
    fn asurl_and_oofurl_derive_the_ews_host() {
        for tag in ["ASUrl", "OOFUrl"] {
            let xml = settings_response(&format!(
                "<Protocol><Type>EXCH</Type>\
                 <{tag}>https://mail.example.com/EWS/Services.wsdl</{tag}>\
                 <SSL>On</SSL></Protocol>"
            ));
            assert_eq!(
                settings_of(&xml).ews.unwrap().url,
                "https://mail.example.com/EWS/Exchange.asmx",
                "{tag}"
            );
        }
    }

    #[test]
    fn imap_and_smtp_ports_and_ssl_combine_into_tls_mode() {
        let xml = settings_response(
            "<Protocol><Type>IMAP</Type><Server>mail.example.com</Server>\
             <Port>993</Port><SSL>on</SSL><LoginName>alice@example.com</LoginName></Protocol>\
             <Protocol><Type>IMAP</Type><Server>ignored.example.com</Server>\
             <Port>143</Port><SSL>off</SSL></Protocol>\
             <Protocol><Type>SMTP</Type><Server>smtp.example.com</Server>\
             <Port>587</Port><SSL>off</SSL><SPA>on</SPA>\
             <AuthRequired>on</AuthRequired></Protocol>\
             <Protocol><Type>SMTP</Type><Server>ignored.example.com</Server>\
             <Port>465</Port><SSL>on</SSL></Protocol>",
        );
        let settings = settings_of(&xml);
        assert_eq!(
            settings.imap,
            Some(DiscoveredEndpoint {
                host: "mail.example.com".into(),
                port: 993,
                tls: TlsMode::Implicit,
                username: Some("alice@example.com".into()),
            })
        );
        assert_eq!(
            settings.smtp,
            Some(DiscoveredEndpoint {
                host: "smtp.example.com".into(),
                port: 587,
                tls: TlsMode::StartTls,
                username: None,
            })
        );
        assert_eq!(tls_mode(993, None, None), Some(TlsMode::Implicit));
        assert_eq!(tls_mode(143, Some("off"), None), Some(TlsMode::StartTls));
        assert_eq!(tls_mode(25, Some("off"), None), Some(TlsMode::StartTls));
        assert_eq!(tls_mode(465, Some("on"), None), Some(TlsMode::Implicit));
        assert_eq!(tls_mode(110, Some("off"), None), Some(TlsMode::StartTls));
        assert_eq!(tls_mode(110, None, None), None);
    }

    #[test]
    fn explicit_encryption_wins_over_port_and_ssl() {
        assert_eq!(tls_mode(993, None, Some("TLS")), Some(TlsMode::StartTls));
        assert_eq!(
            tls_mode(465, Some("off"), Some("SSL")),
            Some(TlsMode::Implicit)
        );
        assert_eq!(tls_mode(993, Some("on"), Some("None")), None);
    }

    #[test]
    fn a_redirect_addr_answer_is_reported_as_a_target() {
        let xml = response(
            "<Action>redirectUrl</Action>\
             <RedirectAddr>https://mail.example.com/autodiscover/autodiscover.xml</RedirectAddr>",
        );
        let Parsed::Redirect(target) =
            parse_settings(&xml, "https://autodiscover.example.com").unwrap()
        else {
            panic!("expected a redirect");
        };
        assert_eq!(
            target,
            "https://mail.example.com/autodiscover/autodiscover.xml"
        );
        // The legacy RedirectUrl element is accepted too.
        let legacy = response(
            "<Action>redirectUrl</Action>\
             <RedirectUrl>https://autodiscover.contoso.com/autodiscover/autodiscover.xml</RedirectUrl>",
        );
        assert!(matches!(
            parse_settings(&legacy, "https://autodiscover.contoso.com").unwrap(),
            Parsed::Redirect(_)
        ));
    }

    #[test]
    fn doctype_and_entity_declarations_are_refused() {
        let entity_case = response("<Action>settings</Action><!ENTITY x \"y\">");
        for xml in [
            "<!DOCTYPE Autodiscover [<!ENTITY xxe SYSTEM \"file:///etc/passwd\">]><Autodiscover/>",
            "<!doctype foo><Autodiscover/>",
            entity_case.as_str(),
        ] {
            assert_eq!(
                parse_settings(xml, "https://autodiscover.example.com").err(),
                Some(DiscoveryError::Unsupported)
            );
        }
    }

    #[test]
    fn plain_http_ews_urls_are_never_offered() {
        assert_eq!(
            ews_url_from("http://mail.example.com/EWS/Exchange.asmx"),
            None
        );
        assert_eq!(ews_url_from("file:///EWS/Exchange.asmx"), None);
        let http_url = settings_response(
            "<Protocol><Type>EXHTTP</Type>\
             <EwsUrl>http://mail.example.com/EWS/Exchange.asmx</EwsUrl>\
             <SSL>On</SSL></Protocol>",
        );
        assert!(matches!(
            parse_settings(&http_url, "https://autodiscover.example.com").unwrap(),
            Parsed::Empty
        ));
        let ssl_off = settings_response(
            "<Protocol><Type>EXPR</Type><Server>mail.example.com</Server>\
             <SSL>Off</SSL><AuthPackage>Ntlm</AuthPackage></Protocol>",
        );
        assert!(matches!(
            parse_settings(&ssl_off, "https://autodiscover.example.com").unwrap(),
            Parsed::Empty
        ));
    }

    #[test]
    fn only_domain_derived_hosts_are_allowed() {
        let domain = "example.com";
        for allowed in [
            "https://example.com/autodiscover/autodiscover.xml",
            "https://autodiscover.example.com/autodiscover/autodiscover.xml",
            "https://mail.example.com/EWS/Exchange.asmx",
            "https://mail.example.com:8443/autodiscover/autodiscover.xml",
            // A redirect may target another subdomain of the same domain.
            "https://other.example.com/autodiscover/autodiscover.xml",
        ] {
            assert!(is_allowed_host(allowed, domain), "{allowed}");
        }
        for rejected in [
            "http://autodiscover.example.com/autodiscover/autodiscover.xml",
            "https://user:secret@autodiscover.example.com/autodiscover/autodiscover.xml",
            "https://example.com.attacker.test/autodiscover/autodiscover.xml",
            "https://attacker.test/autodiscover/autodiscover.xml",
            "file:///etc/passwd",
            "not a url",
            "https://",
        ] {
            assert!(!is_allowed_host(rejected, domain), "{rejected}");
            assert_eq!(
                checked_url(rejected, domain).err(),
                Some(DiscoveryError::Unsupported),
                "{rejected}"
            );
        }
    }

    #[test]
    fn auth_package_is_case_insensitive_and_accepts_multiple_values() {
        for package in [
            "Ntlm",
            "nTlM",
            "Negotiate",
            "NTLM Negotiate",
            "Kerberos, nEgOtIaTe, Basic",
            "Ntlm,Basic",
        ] {
            let xml = settings_response(&format!(
                "<Protocol><Type>EXHTTP</Type><Server>mail.example.com</Server>\
                 <SSL>On</SSL><AuthPackage>{package}</AuthPackage></Protocol>"
            ));
            assert_eq!(
                settings_of(&xml).ews.unwrap().auth,
                EwsAuth::Ntlm,
                "{package}"
            );
        }
    }

    #[test]
    fn basic_only_auth_package_is_basic() {
        for package in ["Basic", "basic", "BASIC", " basic "] {
            let xml = settings_response(&format!(
                "<Protocol><Type>EXHTTP</Type><Server>mail.example.com</Server>\
                 <SSL>On</SSL><AuthPackage>{package}</AuthPackage></Protocol>"
            ));
            assert_eq!(
                settings_of(&xml).ews.unwrap().auth,
                EwsAuth::Basic,
                "{package}"
            );
        }
        // AuthRequired may carry the scheme instead of AuthPackage.
        let xml = settings_response(
            "<Protocol><Type>EXCH</Type><Server>mail.example.com</Server>\
             <SSL>On</SSL><AuthRequired>Basic</AuthRequired></Protocol>",
        );
        assert_eq!(settings_of(&xml).ews.unwrap().auth, EwsAuth::Basic);
    }

    #[test]
    fn empty_and_malformed_xml_are_not_settings() {
        for xml in [
            "",
            "   ",
            "<Autodiscover>",
            "<Account><Action>settings</Action><Protocol><Type>IMAP</Type></Protocol></Account>",
            "<Account><Action>settings</Action><Protocol><Type>POP3</Type>\
             <Server>mail.example.com</Server></Protocol></Account>",
            "<Account><Action>settings</Action><Protocol><Type>IMAP</Type>\
             <Server>mail.example.com/path</Server><Port>993</Port><SSL>on</SSL></Protocol></Account>",
        ] {
            assert!(
                matches!(
                    parse_settings(xml, "https://autodiscover.example.com").unwrap(),
                    Parsed::Empty
                ),
                "{xml}"
            );
        }
        let error = format!(
            "<Autodiscover xmlns=\"{RESPONSE_SCHEMA}\"><Response><Error>\
             <ErrorCode>ErrorInvalidUser</ErrorCode><Message>private-server-text</Message>\
             </Error></Response></Autodiscover>"
        );
        assert!(matches!(
            parse_settings(&error, "https://autodiscover.example.com").unwrap(),
            Parsed::Rejected
        ));
        let too_large = "x".repeat(MAX_RESPONSE_BYTES + 1);
        assert_eq!(
            parse_settings(&too_large, "https://autodiscover.example.com").err(),
            Some(DiscoveryError::Unsupported)
        );
    }

    #[test]
    fn source_is_the_origin_without_query_or_credentials() {
        let url = checked_url(
            "https://mail.example.com:8443/autodiscover/autodiscover.xml?verbose=private",
            "example.com",
        )
        .unwrap();
        assert_eq!(origin(&url), "https://mail.example.com:8443");
        let xml = settings_response(
            "<Protocol><Type>IMAP</Type><Server>mail.example.com</Server>\
             <Port>993</Port><SSL>on</SSL></Protocol>",
        );
        assert_eq!(
            settings_of(&xml).source.as_deref(),
            Some("https://autodiscover.example.com")
        );
    }

    #[test]
    fn prefixed_and_self_closing_tags_are_parsed() {
        let xml = format!(
            "<?xml version=\"1.0\"?>\
             <a:Autodiscover xmlns:a=\"{RESPONSE_SCHEMA}\"><a:Response><a:Account>\
             <a:Action>settings</a:Action>\
             <a:Protocol><a:Type>IMAP</a:Type><a:Server>mail.example.com</a:Server>\
             <a:Port>993</a:Port><a:SSL>on</a:SSL><a:LoginName/></a:Protocol>\
             </a:Account></a:Response></a:Autodiscover>"
        );
        assert_eq!(
            settings_of(&xml).imap,
            Some(DiscoveredEndpoint {
                host: "mail.example.com".into(),
                port: 993,
                tls: TlsMode::Implicit,
                username: None,
            })
        );
    }

    #[test]
    fn a_type_attribute_is_accepted_as_well_as_a_type_element() {
        let xml = response(
            "<Action>settings</Action>\
             <Protocol Type=\"IMAP\"><Server>mail.example.com</Server>\
             <Port>993</Port><SSL>on</SSL></Protocol>",
        );
        let settings = settings_of(&xml);
        assert_eq!(settings.imap.unwrap().port, 993);
    }

    #[test]
    fn www_authenticate_is_read_as_a_fixed_scheme() {
        let mut headers = hyper::HeaderMap::new();
        headers.append(header::WWW_AUTHENTICATE, "Negotiate, NTLM".parse().unwrap());
        assert_eq!(auth_from_headers(&headers), Some(EwsAuth::Ntlm));

        let mut headers = hyper::HeaderMap::new();
        headers.append(
            header::WWW_AUTHENTICATE,
            "Basic realm=\"private\"".parse().unwrap(),
        );
        assert_eq!(auth_from_headers(&headers), Some(EwsAuth::Basic));

        let mut headers = hyper::HeaderMap::new();
        headers.append(
            header::WWW_AUTHENTICATE,
            "Digest realm=\"private\"".parse().unwrap(),
        );
        assert_eq!(auth_from_headers(&headers), None);
    }

    #[test]
    fn credentials_are_never_rendered_in_diagnostics() {
        let password = SecretValue::new("private-password-marker".into());
        let credentials = Credentials {
            username: "alice@example.com",
            password: &password,
        };
        let header = basic_authorization(credentials.username, credentials.password);
        assert!(header.starts_with("Basic "));
        let decoded = STANDARD
            .decode(header.trim_start_matches("Basic "))
            .unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            "alice@example.com:private-password-marker"
        );
        assert!(!format!("{:?}", DiscoveryError::Rejected).contains("private-password-marker"));
    }
}
