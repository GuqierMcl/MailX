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
}
impl LoopbackCallback {
    pub(crate) async fn bind() -> Result<Self, OAuthError> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| OAuthError::Network)?;
        let port = listener
            .local_addr()
            .map_err(|_| OAuthError::Network)?
            .port();
        let path = oauth2::CsrfToken::new_random();
        let redirect = Url::parse(&format!("http://127.0.0.1:{port}/oauth/{}", path.secret()))
            .map_err(|_| OAuthError::InvalidRegistration)?;
        Ok(Self { listener, redirect })
    }
    pub(crate) fn redirect_uri(&self) -> &Url {
        &self.redirect
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
    let body = "Authorization received. Return to application.";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|_| OAuthError::Network)?;
    Ok(callback)
}
