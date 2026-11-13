//! A small HTTP/1.1 client for user-configured services (AI providers, the
//! models.dev catalog). It speaks `http` or `https` on any port over a
//! [`NetRoute`]: a failed proxy is an error, never a direct connection. A
//! redirect is not followed; the 3xx answer is returned like any other status.
//! Everything is bounded: one total timeout and a cap on the answer's size.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    HeaderMap, Method, Request,
    header::{self, HeaderName, HeaderValue},
};
use hyper_util::rt::TokioIo;
use url::{Host, Url};

use super::{NetError, NetRoute};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HttpError {
    /// Could not connect, TLS failed, or the connection broke.
    Network,
    Proxy,
    Timeout,
    /// The answer is larger than the cap.
    TooLarge,
    /// The URL or a header is not something that can be sent.
    Invalid,
}

pub(crate) struct HttpRequest<'a> {
    pub method: Method,
    /// `http` or `https`; the path and query are sent as given.
    pub url: &'a Url,
    pub route: &'a NetRoute,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

#[derive(Debug)]
pub(crate) struct HttpReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

/// A header value that must not show up in debug output.
pub(crate) fn sensitive_header(value: &str) -> Option<HeaderValue> {
    let mut value = HeaderValue::from_str(value).ok()?;
    value.set_sensitive(true);
    Some(value)
}

/// Aborts the connection task when the request future is dropped.
struct Connection(tokio::task::JoinHandle<()>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn net_error(error: NetError) -> HttpError {
    match error {
        NetError::Proxy(_) => HttpError::Proxy,
        NetError::Timeout => HttpError::Timeout,
        _ => HttpError::Network,
    }
}

pub(crate) async fn send(request: HttpRequest<'_>) -> Result<HttpReply, HttpError> {
    let timeout = request.timeout;
    tokio::time::timeout(timeout, send_inner(request))
        .await
        .map_err(|_| HttpError::Timeout)?
}

async fn send_inner(request: HttpRequest<'_>) -> Result<HttpReply, HttpError> {
    let HttpRequest {
        method,
        url,
        route,
        headers,
        body,
        max_response_bytes,
        ..
    } = request;
    let tls = match url.scheme() {
        "https" => true,
        "http" => false,
        _ => return Err(HttpError::Invalid),
    };
    let port = url.port_or_known_default().ok_or(HttpError::Invalid)?;
    // `Url` keeps IPv6 literals in brackets; sockets and TLS names want them bare.
    let (host, authority_host) = match url.host().ok_or(HttpError::Invalid)? {
        Host::Ipv6(address) => (address.to_string(), format!("[{address}]")),
        other => (other.to_string(), other.to_string()),
    };
    let default_port = if tls { 443 } else { 80 };
    let authority = if port == default_port {
        authority_host
    } else {
        format!("{authority_host}:{port}")
    };
    let mut target = url.path().to_owned();
    if let Some(query) = url.query() {
        target.push('?');
        target.push_str(query);
    }

    // A failed proxy is an error here, never a direct connection.
    let mut stream = super::connect(route, &host, port)
        .await
        .map_err(net_error)?;
    if tls {
        stream = super::upgrade_tls(stream, &host).await.map_err(net_error)?;
    }
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_buf_size(64 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| HttpError::Network)?;
    let _connection = Connection(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let mut builder = Request::builder()
        .method(method)
        .uri(target)
        .header(header::HOST, authority)
        .header(header::ACCEPT, "application/json")
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::USER_AGENT, "MailX");
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let mut request = builder
        .body(Full::new(Bytes::from(body.unwrap_or_default())))
        .map_err(|_| HttpError::Invalid)?;
    for (name, value) in headers {
        request.headers_mut().insert(name, value);
    }
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| HttpError::Network)?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| HttpError::Network)?;
        if let Some(data) = frame.data_ref() {
            if bytes.len() + data.len() > max_response_bytes {
                return Err(HttpError::TooLarge);
            }
            bytes.extend_from_slice(data);
        }
    }
    Ok(HttpReply {
        status,
        headers,
        body: bytes,
    })
}
