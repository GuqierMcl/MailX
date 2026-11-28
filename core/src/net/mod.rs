//! Outbound connections for mail protocols.
//!
//! Every connection follows the account's route: direct TCP, or a SOCKS5 /
//! HTTP CONNECT proxy that resolves the target host name itself. A proxy
//! failure is reported as such and never falls back to a direct connection.
//! TLS is verified against the operating system trust store.
//!
//! rustls is the TLS stack. Some mail servers only offer cipher suites rustls
//! deliberately omits (finite-field DHE, static RSA key exchange); IMAP and
//! SMTP connections to them fall back to the operating system library through
//! `with_tls_fallback`, over the same route, only when the server rejects
//! every parameter rustls offered.

pub(crate) mod http;
pub(crate) mod images;
#[cfg(test)]
mod live_tls;
mod proxy;
#[cfg(test)]
pub(crate) mod test_support;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{AlertDescription, ClientConfig};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_native_tls::native_tls;
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
    /// Explicit account consent for HTTPS image target DNS at this proxy.
    /// Mail protocol connections do not consume this image-only permission.
    pub image_proxy_dns: bool,
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

/// Reason recorded when the server and rustls share no TLS version or cipher
/// suite. Kept inside `Tls` so every existing mapping treats it as a TLS failure.
const TLS_INCOMPATIBLE: &str = "no TLS parameters in common with the server";

impl NetError {
    /// The server rejected every TLS version and cipher suite rustls offered.
    /// Certificate, timeout and connection failures never qualify.
    pub(crate) fn is_tls_incompatible(&self) -> bool {
        matches!(self, Self::Tls(reason) if reason == TLS_INCOMPATIBLE)
    }
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

/// Which library performs a mail protocol TLS handshake. Chosen per endpoint by
/// `with_tls_fallback`, never by the account's brand or address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TlsStack {
    Rustls,
    /// Secure Transport on macOS, SChannel on Windows, OpenSSL elsewhere.
    /// Certificates are verified by the platform; TLS 1.2 is the floor.
    Platform,
}

/// Runs a mail protocol TLS handshake for `host` with the requested stack.
pub(crate) async fn upgrade_tls_on(
    stream: BoxedStream,
    host: &str,
    stack: TlsStack,
) -> Result<BoxedStream, NetError> {
    match stack {
        TlsStack::Rustls => upgrade_tls(stream, host).await,
        TlsStack::Platform => platform_handshake(stream, host).await,
    }
}

/// TLS stack learned per mail endpoint (lower-case host, port) in this process.
fn learned_stacks() -> &'static Mutex<HashMap<(String, u16), TlsStack>> {
    static LEARNED: OnceLock<Mutex<HashMap<(String, u16), TlsStack>>> = OnceLock::new();
    LEARNED.get_or_init(Default::default)
}

fn learned_stack(key: &(String, u16)) -> Option<TlsStack> {
    learned_stacks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(key)
        .copied()
}

fn learn_stack(key: (String, u16), stack: TlsStack) {
    let mut stacks = learned_stacks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match stack {
        // A rustls success pins the endpoint against later downgrades.
        TlsStack::Rustls => {
            stacks.insert(key, TlsStack::Rustls);
        }
        // Platform fallback must never overwrite an already-recorded rustls success.
        TlsStack::Platform => {
            stacks.entry(key).or_insert(TlsStack::Platform);
        }
    }
}

/// Runs one mail connection attempt (`attempt` connects along the account
/// route and performs the TLS handshake with the stack it is given).
///
/// An endpoint starts with rustls. Only if `incompatible` says the server
/// shared no TLS parameters with rustls is the whole attempt repeated once on
/// the platform stack, which then serves that endpoint for the rest of the
/// process. An endpoint that has completed a rustls attempt never falls back,
/// so a forged alert cannot downgrade a server known to work with rustls.
pub(crate) async fn with_tls_fallback<T, E, Fut>(
    host: &str,
    port: u16,
    mut attempt: impl FnMut(TlsStack) -> Fut,
    incompatible: impl Fn(&E) -> bool,
) -> Result<T, E>
where
    Fut: Future<Output = Result<T, E>>,
{
    let key = (host.to_ascii_lowercase(), port);
    let known = learned_stack(&key);
    match attempt(known.unwrap_or(TlsStack::Rustls)).await {
        Ok(value) => {
            if known.is_none() {
                learn_stack(key, TlsStack::Rustls);
            }
            Ok(value)
        }
        Err(error)
            if known.is_none()
                && incompatible(&error)
                && learned_stack(&key) != Some(TlsStack::Rustls) =>
        {
            let result = attempt(TlsStack::Platform).await;
            if result.is_ok() {
                learn_stack(key, TlsStack::Platform);
            }
            result
        }
        Err(error) => Err(error),
    }
}

fn platform_tls_connector() -> Result<tokio_native_tls::TlsConnector, NetError> {
    static CONNECTOR: OnceLock<Result<native_tls::TlsConnector, &'static str>> = OnceLock::new();
    CONNECTOR
        .get_or_init(|| {
            native_tls::TlsConnector::builder()
                .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
                .build()
                .map_err(|_| "platform TLS unavailable")
        })
        .clone()
        .map(tokio_native_tls::TlsConnector::from)
        .map_err(|reason| NetError::Tls(reason.into()))
}

async fn platform_handshake(stream: BoxedStream, host: &str) -> Result<BoxedStream, NetError> {
    let connector = platform_tls_connector()?;
    let handshake = connector.connect(host, stream);
    let stream = timeout(CONNECT_TIMEOUT, handshake)
        .await
        .map_err(|_| NetError::Timeout)?
        // Platform diagnostics can name peer certificates, like rustls ones.
        .map_err(|_| tls_handshake_error_message())?;
    Ok(Box::new(stream))
}

/// Runs a TLS handshake for `host` over an established stream.
pub(crate) async fn upgrade_tls(stream: BoxedStream, host: &str) -> Result<BoxedStream, NetError> {
    handshake(stream, host, tls_config()?).await
}

/// EWS binds NTLM to the verified TLS server certificate. Capture it before
/// erasing the TLS stream type; the certificate belongs to this exact tunnel.
pub(crate) async fn upgrade_tls_with_peer_certificate(
    stream: BoxedStream,
    host: &str,
) -> Result<(BoxedStream, Vec<u8>), NetError> {
    let stream = tls_handshake(stream, host, tls_config()?).await?;
    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .ok_or_else(|| NetError::Tls("server certificate unavailable".into()))?
        .as_ref()
        .to_vec();
    Ok((Box::new(stream), certificate))
}

/// Test-only injection point: the same handshake with a test trust store.
/// There is deliberately no production counterpart.
#[cfg(test)]
pub(crate) async fn upgrade_tls_with(
    stream: BoxedStream,
    host: &str,
    config: Arc<ClientConfig>,
) -> Result<BoxedStream, NetError> {
    handshake(stream, host, config).await
}

async fn handshake(
    stream: BoxedStream,
    host: &str,
    config: Arc<ClientConfig>,
) -> Result<BoxedStream, NetError> {
    Ok(Box::new(tls_handshake(stream, host, config).await?))
}

async fn tls_handshake(
    stream: BoxedStream,
    host: &str,
    config: Arc<ClientConfig>,
) -> Result<tokio_rustls::client::TlsStream<BoxedStream>, NetError> {
    let name = ServerName::try_from(host.to_owned())
        .map_err(|_| NetError::Tls("invalid TLS server name".into()))?;
    let handshake = TlsConnector::from(config).connect(name, stream);
    let stream = timeout(CONNECT_TIMEOUT, handshake)
        .await
        .map_err(|_| NetError::Timeout)?
        .map_err(tls_handshake_error)?;
    Ok(stream)
}

fn tls_handshake_error(error: std::io::Error) -> NetError {
    // rustls and platform diagnostics can contain names from peer certificates;
    // only the fact that negotiation found nothing in common is kept.
    let incompatible = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|error| {
            matches!(
                error,
                rustls::Error::PeerIncompatible(_)
                    | rustls::Error::AlertReceived(
                        AlertDescription::HandshakeFailure
                            | AlertDescription::InsufficientSecurity
                            | AlertDescription::ProtocolVersion
                    )
            )
        });
    if incompatible {
        NetError::Tls(TLS_INCOMPATIBLE.into())
    } else {
        tls_handshake_error_message()
    }
}

fn tls_handshake_error_message() -> NetError {
    NetError::Tls("handshake or certificate verification failed".into())
}

fn tls_config() -> Result<Arc<ClientConfig>, NetError> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, &'static str>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|_| "TLS protocol configuration unavailable")?
                .with_platform_verifier()
                .map(|builder| Arc::new(builder.with_no_client_auth()))
                .map_err(|_| "platform certificate verifier unavailable")
        })
        .clone()
        .map_err(|reason| NetError::Tls(reason.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_diagnostics_do_not_expose_peer_names_or_platform_details() {
        let certificate_error =
            rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidForNameContext {
                expected: ServerName::try_from("expected.example".to_owned()).unwrap(),
                presented: vec!["private-password-marker.example".into()],
            });
        let general_error = rustls::Error::General("private-password-marker".into());
        for error in [
            std::io::Error::new(std::io::ErrorKind::InvalidData, certificate_error),
            std::io::Error::new(std::io::ErrorKind::InvalidData, general_error),
            std::io::Error::other("platform private-password-marker"),
        ] {
            // Prove the source actually contains the sensitive diagnostic.
            assert!(error.to_string().contains("private-password-marker"));
            let sanitized = tls_handshake_error(error);
            assert_eq!(
                sanitized.to_string(),
                "TLS failed: handshake or certificate verification failed"
            );
            assert!(!format!("{sanitized:?}").contains("private-password-marker"));
        }
    }

    #[test]
    fn only_negotiation_failures_count_as_tls_incompatible() {
        let wrap = |error: rustls::Error| {
            tls_handshake_error(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        };
        for error in [
            rustls::Error::AlertReceived(AlertDescription::HandshakeFailure),
            rustls::Error::AlertReceived(AlertDescription::InsufficientSecurity),
            rustls::Error::AlertReceived(AlertDescription::ProtocolVersion),
            rustls::Error::PeerIncompatible(rustls::PeerIncompatible::NoCipherSuitesInCommon),
        ] {
            let mapped = wrap(error.clone());
            assert!(mapped.is_tls_incompatible(), "{error:?}");
            assert!(matches!(mapped, NetError::Tls(_)), "{error:?}");
        }
        for error in [
            rustls::Error::AlertReceived(AlertDescription::BadCertificate),
            rustls::Error::AlertReceived(AlertDescription::UnknownCA),
            rustls::Error::InvalidCertificate(rustls::CertificateError::Expired),
            rustls::Error::General("handshake_failure".into()),
        ] {
            assert!(!wrap(error.clone()).is_tls_incompatible(), "{error:?}");
        }
        for error in [
            NetError::Timeout,
            NetError::Connect("refused".into()),
            NetError::Proxy("handshake failure".into()),
            tls_handshake_error(std::io::Error::other("platform")),
        ] {
            assert!(!error.is_tls_incompatible(), "{error:?}");
        }
    }

    /// Runs `with_tls_fallback` with scripted per-stack results and returns
    /// the stacks attempted, in order.
    async fn fallback(
        host: &str,
        rustls: Result<(), NetError>,
        platform: Result<(), NetError>,
    ) -> (Result<(), NetError>, Vec<TlsStack>) {
        let tried = Mutex::new(Vec::new());
        let result = with_tls_fallback(
            host,
            993,
            |stack| {
                tried.lock().unwrap().push(stack);
                let result = match stack {
                    TlsStack::Rustls => rustls.clone(),
                    TlsStack::Platform => platform.clone(),
                };
                async move { result }
            },
            NetError::is_tls_incompatible,
        )
        .await;
        (result, tried.into_inner().unwrap())
    }

    fn incompatible() -> Result<(), NetError> {
        Err(NetError::Tls(TLS_INCOMPATIBLE.into()))
    }

    #[tokio::test]
    async fn fallback_retries_once_and_remembers_the_platform_stack() {
        use TlsStack::{Platform, Rustls};
        let host = "Fallback-Learns.example";
        let (result, tried) = fallback(host, incompatible(), Ok(())).await;
        assert!(result.is_ok());
        assert_eq!(tried, [Rustls, Platform]);
        // Learned per endpoint, case-insensitively: no second rustls attempt.
        let (result, tried) = fallback("fallback-learns.example", incompatible(), Ok(())).await;
        assert!(result.is_ok());
        assert_eq!(tried, [Platform]);
    }

    #[tokio::test]
    async fn fallback_keeps_rustls_when_it_ever_worked_or_the_failure_is_not_negotiation() {
        use TlsStack::{Platform, Rustls};
        let (_, tried) = fallback("fallback-sticky.example", Ok(()), Ok(())).await;
        assert_eq!(tried, [Rustls]);
        let (result, tried) = fallback("fallback-sticky.example", incompatible(), Ok(())).await;
        assert!(result.unwrap_err().is_tls_incompatible());
        assert_eq!(tried, [Rustls]);

        for error in [
            NetError::Tls("handshake or certificate verification failed".into()),
            NetError::Timeout,
            NetError::Proxy("refused".into()),
        ] {
            let (result, tried) =
                fallback("fallback-other.example", Err(error.clone()), Ok(())).await;
            assert_eq!(result, Err(error));
            assert_eq!(tried, [Rustls]);
        }

        // A failed retry teaches nothing: the next connection starts on rustls.
        let (result, tried) = fallback(
            "fallback-fails.example",
            incompatible(),
            Err(NetError::Timeout),
        )
        .await;
        assert_eq!(result, Err(NetError::Timeout));
        assert_eq!(tried, [Rustls, Platform]);
        let (_, tried) = fallback(
            "fallback-fails.example",
            incompatible(),
            Err(NetError::Timeout),
        )
        .await;
        assert_eq!(tried, [Rustls, Platform]);
    }

    #[tokio::test]
    async fn concurrent_rustls_success_blocks_and_never_loses_to_platform_fallback() {
        use TlsStack::{Platform, Rustls};
        use tokio::sync::oneshot;

        // Case 1: both start with no learned stack; A succeeds on rustls before B
        // finishes its failing rustls attempt -> B must not retry on Platform.
        let host1 = "fallback-race-block.example";
        let (b_started_tx, b_started_rx) = oneshot::channel();
        let (a_done_tx, a_done_rx) = oneshot::channel();
        let b_tried = Arc::new(Mutex::new(Vec::new()));
        let b_tried_task = b_tried.clone();
        let b_started = Mutex::new(Some(b_started_tx));
        let a_done = Mutex::new(Some(a_done_rx));
        let task_b = tokio::spawn(async move {
            with_tls_fallback(
                host1,
                993,
                move |stack| {
                    b_tried_task.lock().unwrap().push(stack);
                    let started = b_started.lock().unwrap().take();
                    let wait = a_done.lock().unwrap().take();
                    async move {
                        if let Some(tx) = started {
                            let _ = tx.send(());
                        }
                        if let Some(rx) = wait {
                            let _ = rx.await;
                        }
                        match stack {
                            Rustls => incompatible(),
                            Platform => Ok(()),
                        }
                    }
                },
                NetError::is_tls_incompatible,
            )
            .await
        });
        b_started_rx.await.unwrap();
        let (a_res, a_tried) = fallback(host1, Ok(()), Ok(())).await;
        assert!(a_res.is_ok());
        assert_eq!(a_tried, [Rustls]);
        let _ = a_done_tx.send(());
        let b_res = task_b.await.unwrap();
        assert!(b_res.unwrap_err().is_tls_incompatible());
        assert_eq!(*b_tried.lock().unwrap(), [Rustls]);

        // Case 2: B already entered its Platform retry when A succeeds on rustls ->
        // B's Platform success must never overwrite the recorded Rustls stack.
        let host2 = "fallback-race-no-overwrite.example";
        let (b_in_platform_tx, b_in_platform_rx) = oneshot::channel();
        let (allow_b_tx, allow_b_rx) = oneshot::channel();
        let b_in_platform = Mutex::new(Some(b_in_platform_tx));
        let allow_b = Mutex::new(Some(allow_b_rx));
        let task_b = tokio::spawn(async move {
            with_tls_fallback(
                host2,
                993,
                move |stack| {
                    let signal = (stack == Platform)
                        .then(|| b_in_platform.lock().unwrap().take())
                        .flatten();
                    let wait = (stack == Platform)
                        .then(|| allow_b.lock().unwrap().take())
                        .flatten();
                    async move {
                        match stack {
                            Rustls => incompatible(),
                            Platform => {
                                if let Some(tx) = signal {
                                    let _ = tx.send(());
                                }
                                if let Some(rx) = wait {
                                    let _ = rx.await;
                                }
                                Ok(())
                            }
                        }
                    }
                },
                NetError::is_tls_incompatible,
            )
            .await
        });
        b_in_platform_rx.await.unwrap();
        let (a_res, a_tried) = fallback(host2, Ok(()), Ok(())).await;
        assert!(a_res.is_ok());
        assert_eq!(a_tried, [Rustls]);
        let _ = allow_b_tx.send(());
        assert!(task_b.await.unwrap().is_ok());

        // Subsequent connections must stay pinned to Rustls.
        let (after, tried) = fallback(host2, incompatible(), Ok(())).await;
        assert!(after.unwrap_err().is_tls_incompatible());
        assert_eq!(tried, [Rustls]);
    }
}
