//! Ephemeral loopback callbacks, independent of desktop/window APIs.
use super::OAuthError;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use url::Url;

pub(crate) struct LoopbackCallback {
    listener: TcpListener,
    redirect: Url,
    redirect_uri: String,
}
impl LoopbackCallback {
    /// Binds an ephemeral IPv4 loopback port. Providers match a loopback
    /// redirect on everything except the port, so `path` is the registered
    /// value (empty for none) and the text is built here, once, to be reused
    /// byte for byte in the authorization and token requests.
    pub(crate) async fn bind(path: &str) -> Result<Self, OAuthError> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| OAuthError::Network)?;
        let port = listener
            .local_addr()
            .map_err(|_| OAuthError::Network)?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}{path}");
        let redirect = Url::parse(&redirect_uri).map_err(|_| OAuthError::InvalidRegistration)?;
        Ok(Self {
            listener,
            redirect,
            redirect_uri,
        })
    }
    /// The exact `redirect_uri` text, without a trailing slash for an empty path.
    pub(crate) fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }
    /// Overall lifetime, individual connection lifetime, and request bytes are
    /// bounded separately: a speculative browser connection cannot monopolize
    /// the listener until the authorization expires. Dropping this future also
    /// drops the listener and releases its ephemeral port.
    pub(crate) async fn receive(self) -> Result<Url, OAuthError> {
        tokio::time::timeout(Duration::from_secs(300), async {
            loop {
                let (stream, peer) = self
                    .listener
                    .accept()
                    .await
                    .map_err(|_| OAuthError::Network)?;
                if !peer.ip().is_loopback() {
                    continue;
                }
                if let Ok(Ok(callback)) = tokio::time::timeout(
                    Duration::from_secs(2),
                    read_callback(stream, &self.redirect),
                )
                .await
                {
                    return Ok(callback);
                }
            }
        })
        .await
        .map_err(|_| OAuthError::Timeout)?
    }
}
/// No script, no network access and no framing; only the inline style below.
const PAGE_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
macro_rules! page {
    ($title_en:literal, $text_en:literal, $title_zh:literal, $text_zh:literal) => {
        concat!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
            "<meta name=\"referrer\" content=\"no-referrer\"><title>MailX</title>",
            "<style>body{font:16px/1.6 system-ui,sans-serif;max-width:28rem;margin:20vh auto;padding:0 1.5rem;color:#222}",
            "h1{font-size:1.25rem;margin:0 0 .25rem}p{margin:0 0 1.5rem;color:#555}",
            "@media(prefers-color-scheme:dark){body{background:#1b1b1b;color:#eee}p{color:#aaa}}</style></head><body>",
            "<h1 lang=\"en\">", $title_en, "</h1><p lang=\"en\">", $text_en, "</p>",
            "<h1 lang=\"zh-CN\">", $title_zh, "</h1><p lang=\"zh-CN\">", $text_zh, "</p>",
            "</body></html>"
        )
    };
}
const PAGE_DONE: &str = page!(
    "Sign-in complete",
    "You can close this tab and return to MailX.",
    "登录完成",
    "可以关闭此页面，返回 MailX。"
);
const PAGE_FAILED: &str = page!(
    "Sign-in was not completed",
    "Return to MailX and try again.",
    "未能完成登录",
    "请返回 MailX 后重试。"
);
async fn read_callback(mut stream: TcpStream, redirect: &Url) -> Result<Url, OAuthError> {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 1024];
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| OAuthError::Network)?;
        if n == 0 || bytes.len() + n > 16384 {
            return Err(OAuthError::InvalidCallback);
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&bytes).map_err(|_| OAuthError::InvalidCallback)?;
    let mut lines = request.split("\r\n");
    let mut words = lines.next().ok_or(OAuthError::InvalidCallback)?.split(' ');
    if words.next() != Some("GET") {
        return Err(OAuthError::InvalidCallback);
    }
    let target = words.next().ok_or(OAuthError::InvalidCallback)?;
    if words.next() != Some("HTTP/1.1")
        || words.next().is_some()
        || !target.starts_with('/')
        || target.starts_with("//")
        || target.contains('#')
    {
        return Err(OAuthError::InvalidCallback);
    }
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.trim())
        .collect();
    let authority = &redirect[url::Position::BeforeHost..url::Position::AfterPort];
    if hosts != [authority] {
        return Err(OAuthError::InvalidCallback);
    }
    let callback = redirect
        .join(target)
        .map_err(|_| OAuthError::InvalidCallback)?;
    if callback.path() != redirect.path() {
        return Err(OAuthError::InvalidCallback);
    }
    let failed = callback.query_pairs().any(|(key, _)| key == "error");
    let body = if failed { PAGE_FAILED } else { PAGE_DONE };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: {PAGE_CSP}\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|_| OAuthError::Network)?;
    Ok(callback)
}
