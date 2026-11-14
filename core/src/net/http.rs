//! A small HTTP/1.1 client for user-configured services (AI providers, the
//! models.dev catalog). It speaks `http` or `https` on any port over a
//! [`NetRoute`]: a failed proxy is an error, never a direct connection. A
//! redirect is not followed; the 3xx answer is returned like any other status.
//! Everything is bounded: one total timeout and a cap on the answer's size.

use std::time::Duration;

use tokio::time::Instant;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
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

/// What `open` hands back: a sender with its connection task guard.
struct Opened {
    sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
    connection: Connection,
    request: Request<Full<Bytes>>,
}

/// Connects over the route (a failed proxy is an error, never a direct
/// connection), upgrades to TLS for `https`, and builds the request.
async fn open(request: HttpRequest<'_>, accept: &'static str) -> Result<Opened, HttpError> {
    let HttpRequest {
        method,
        url,
        route,
        headers,
        body,
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

    let mut stream = super::connect(route, &host, port)
        .await
        .map_err(net_error)?;
    if tls {
        stream = super::upgrade_tls(stream, &host).await.map_err(net_error)?;
    }
    let (sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_buf_size(64 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| HttpError::Network)?;
    let connection = Connection(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let mut builder = Request::builder()
        .method(method)
        .uri(target)
        .header(header::HOST, authority)
        .header(header::ACCEPT, accept)
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
    Ok(Opened {
        sender,
        connection,
        request,
    })
}

async fn send_inner(request: HttpRequest<'_>) -> Result<HttpReply, HttpError> {
    let max_response_bytes = request.max_response_bytes;
    let Opened {
        mut sender,
        connection: _connection,
        request,
    } = open(request, "application/json").await?;
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

/// Bounds for one streamed answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamLimits {
    /// Connecting, TLS and the response headers.
    pub first_byte: Duration,
    /// The longest gap between two body chunks.
    pub idle: Duration,
    /// The whole answer, from the start of the request.
    pub total: Duration,
    /// The body's size across all chunks.
    pub max_bytes: usize,
}

/// A response whose body is read chunk by chunk. The connection task lives
/// as long as this value; dropping it (mid-stream or not) aborts the task and
/// closes the socket, which is how a stream is cancelled.
pub(crate) struct StreamingReply {
    pub status: u16,
    body: Incoming,
    limits: StreamLimits,
    deadline: Instant,
    received: usize,
    _connection: Connection,
}

impl StreamingReply {
    /// The next piece of body, `None` at the end. Waits at most the idle
    /// limit, and never past the total limit (`Timeout`); a body beyond
    /// `max_bytes` is `TooLarge`.
    pub(crate) async fn next_chunk(&mut self) -> Result<Option<Bytes>, HttpError> {
        let wait_until = self.deadline.min(Instant::now() + self.limits.idle);
        loop {
            let frame = tokio::time::timeout_at(wait_until, self.body.frame())
                .await
                .map_err(|_| HttpError::Timeout)?;
            let Some(frame) = frame else {
                return Ok(None);
            };
            let frame = frame.map_err(|_| HttpError::Network)?;
            // Trailers carry nothing we use.
            if let Ok(data) = frame.into_data() {
                if data.is_empty() {
                    continue;
                }
                self.received += data.len();
                if self.received > self.limits.max_bytes {
                    return Err(HttpError::TooLarge);
                }
                return Ok(Some(data));
            }
        }
    }
}

/// Like [`send`], but the body is delivered as it arrives (`Accept:
/// text/event-stream`). `request.timeout` and `request.max_response_bytes` are
/// not used: `limits` bounds the answer. The body of a non-2xx answer is not
/// read; the caller only needs the status.
pub(crate) async fn send_streaming(
    request: HttpRequest<'_>,
    limits: StreamLimits,
) -> Result<StreamingReply, HttpError> {
    let started = Instant::now();
    let first_byte = limits.first_byte.min(limits.total);
    let (response, connection) = tokio::time::timeout(first_byte, async {
        let Opened {
            mut sender,
            connection,
            request,
        } = open(request, "text/event-stream").await?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| HttpError::Network)?;
        Ok::<_, HttpError>((response, connection))
    })
    .await
    .map_err(|_| HttpError::Timeout)??;
    Ok(StreamingReply {
        status: response.status().as_u16(),
        body: response.into_body(),
        limits,
        deadline: started + limits.total,
        received: 0,
        _connection: connection,
    })
}
