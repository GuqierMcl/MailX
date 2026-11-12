//! One HTTPS POST (JSON unless the caller overrides the content type) over the translation route. Everything except the
//! request body is bounded: 30 s in total, 4 MiB of answer.
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request,
    header::{self, HeaderName, HeaderValue},
};
use hyper_util::rt::TokioIo;

use crate::{
    ErrorCode,
    net::{self, NetError, NetRoute},
};

const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Base {
    host: String,
    port: u16,
    /// Never true outside tests: the only constructor for it is test-only.
    plain: bool,
}

impl Base {
    pub(super) fn https(host: &str) -> Self {
        Self {
            host: host.to_owned(),
            port: 443,
            plain: false,
        }
    }

    /// Plain HTTP to `127.0.0.1:port`. Exists only for tests.
    #[cfg(test)]
    pub(crate) fn plain(port: u16) -> Self {
        Self {
            host: "127.0.0.1".into(),
            port,
            plain: true,
        }
    }

    fn authority(&self) -> String {
        if self.port == 443 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

pub(super) struct Post<'a> {
    pub base: &'a Base,
    pub route: &'a NetRoute,
    /// Path and query.
    pub path: &'a str,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: Vec<u8>,
}

pub(super) struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// A header value that must not show up in debug output.
pub(super) fn secret_header(value: &str) -> Result<HeaderValue, ErrorCode> {
    let mut value = HeaderValue::from_str(value).map_err(|_| ErrorCode::TranslationKeyRejected)?;
    value.set_sensitive(true);
    Ok(value)
}

/// Aborts the connection task when the request future is dropped.
struct Connection(tokio::task::JoinHandle<()>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn net_error(error: NetError) -> ErrorCode {
    match error {
        NetError::Proxy(_) => ErrorCode::TranslationProxy,
        NetError::Timeout => ErrorCode::TranslationTimeout,
        _ => ErrorCode::TranslationNetwork,
    }
}

pub(super) async fn post(call: Post<'_>) -> Result<Reply, ErrorCode> {
    tokio::time::timeout(TOTAL_TIMEOUT, post_inner(call))
        .await
        .map_err(|_| ErrorCode::TranslationTimeout)?
}

async fn post_inner(call: Post<'_>) -> Result<Reply, ErrorCode> {
    let Post {
        base,
        route,
        path,
        headers,
        body,
    } = call;
    // A failed proxy is an error here, never a direct connection.
    let mut stream = net::connect(route, &base.host, base.port)
        .await
        .map_err(net_error)?;
    if !base.plain {
        stream = net::upgrade_tls(stream, &base.host)
            .await
            .map_err(net_error)?;
    }
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_buf_size(64 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| ErrorCode::TranslationNetwork)?;
    let _connection = Connection(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let mut request = Request::post(path)
        .header(header::HOST, base.authority())
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, "MailX")
        .body(Full::new(Bytes::from(body)))
        .map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    for (name, value) in headers {
        request.headers_mut().insert(name, value);
    }
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| ErrorCode::TranslationNetwork)?;
    let status = response.status().as_u16();
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ErrorCode::TranslationNetwork)?;
        if let Some(data) = frame.data_ref() {
            if bytes.len() + data.len() > MAX_RESPONSE_BYTES {
                return Err(ErrorCode::TranslationInvalidResponse);
            }
            bytes.extend_from_slice(data);
        }
    }
    Ok(Reply {
        status,
        body: bytes,
    })
}

/// The mapping every service shares; adapters handle their own statuses first.
pub(super) fn common_status_error(status: u16) -> ErrorCode {
    match status {
        401 | 403 => ErrorCode::TranslationKeyRejected,
        429 => ErrorCode::TranslationRateLimited,
        413 => ErrorCode::TranslationTooLarge,
        500..=599 => ErrorCode::TranslationUnavailable,
        _ => ErrorCode::TranslationInvalidResponse,
    }
}

/// For the keyless browser channels: there is no key to reject, so a refusal
/// means the channel itself is closed, blocked or gone.
pub(super) fn keyless_status_error(status: u16) -> ErrorCode {
    match status {
        401 | 403 | 404 | 410 => ErrorCode::TranslationUnavailable,
        _ => common_status_error(status),
    }
}
