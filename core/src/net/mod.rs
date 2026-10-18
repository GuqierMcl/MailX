//! Outbound connections for mail protocols.
//!
//! Every connection follows the account's route: direct TCP, or a SOCKS5 /
//! HTTP CONNECT proxy that resolves the target host name itself. A proxy
//! failure is reported as such and never falls back to a direct connection.
//! TLS is verified against the operating system trust store.

mod proxy;

use std::fmt;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::accounts::types::ProxyKind;
use crate::secrets::SecretValue;

/// Initial connection budget from docs/architecture/mail-sync.md §2.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A byte stream a protocol client can run over: plain TCP, a proxy tunnel, or TLS on either.
pub(crate) trait MailStream: AsyncRead + AsyncWrite + Unpin + Send + fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + fmt::Debug> MailStream for T {}

pub(crate) type BoxedStream = Box<dyn MailStream>;

#[derive(Debug)]
pub(crate) struct ProxyRoute {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub auth: Option<(String, SecretValue)>,
}

/// How to reach a server. `None` connects directly.
#[derive(Debug, Default)]
pub(crate) struct NetRoute {
    pub proxy: Option<ProxyRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetError {
    /// The direct connection failed or timed out.
    Connect(String),
    /// The proxy refused, failed authentication, or could not reach the target.
    Proxy(String),
    /// The TLS handshake or certificate verification failed.
    Tls(String),
    Timeout,
}

impl fmt::Display for NetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(reason) => write!(formatter, "connection failed: {reason}"),
            Self::Proxy(reason) => write!(formatter, "proxy failed: {reason}"),
            Self::Tls(reason) => write!(formatter, "TLS failed: {reason}"),
            Self::Timeout => formatter.write_str("connection timed out"),
        }
    }
}

/// Opens a plain stream to `host:port` along `route`.
pub(crate) async fn connect(
    route: &NetRoute,
    host: &str,
    port: u16,
) -> Result<BoxedStream, NetError> {
    let attempt = async {
        match &route.proxy {
            None => TcpStream::connect((host, port))
                .await
                .map(|stream| Box::new(stream) as BoxedStream)
                .map_err(|error| NetError::Connect(error.to_string())),
            Some(proxy) => proxy::connect(proxy, host, port).await,
        }
    };
    timeout(CONNECT_TIMEOUT, attempt)
        .await
        .map_err(|_| NetError::Timeout)?
}

/// Runs a TLS handshake for `host` over an established stream.
pub(crate) async fn upgrade_tls(stream: BoxedStream, host: &str) -> Result<BoxedStream, NetError> {
    let config = tls_config()?;
    let name =
        ServerName::try_from(host.to_owned()).map_err(|error| NetError::Tls(error.to_string()))?;
    let handshake = TlsConnector::from(config).connect(name, stream);
    let stream = timeout(CONNECT_TIMEOUT, handshake)
        .await
        .map_err(|_| NetError::Timeout)?
        .map_err(|error| NetError::Tls(error.to_string()))?;
    Ok(Box::new(stream))
}

fn tls_config() -> Result<Arc<ClientConfig>, NetError> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|error| error.to_string())?
                .with_platform_verifier()
                .map(|builder| Arc::new(builder.with_no_client_auth()))
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(NetError::Tls)
}
