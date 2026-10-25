//! Resolve once, then give the connector only the checked address snapshot.
use super::{ImageFetchError, public_ip};
use crate::net::{BoxedStream, NetError, NetRoute};
use hyper_util::client::legacy::connect::{HttpConnector, dns::Name};
use std::{
    future::{Ready, ready},
    io,
    net::{IpAddr, SocketAddr},
    task::{Context, Poll},
    time::Duration,
};
use tower_service::Service;
use url::Url;

const MAX_RESOLVED_ADDRESSES: usize = 64;
const MAX_CANDIDATES: usize = 16;
const HAPPY_EYEBALLS_DELAY: Duration = Duration::from_millis(300);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Target {
    Pinned(Vec<IpAddr>),
    ProxyDomain,
}

fn local_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    !host.contains('.')
        || ["localhost", "local", "lan", "internal", "home.arpa"]
            .iter()
            .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

pub(super) async fn target(route: &NetRoute, url: &Url) -> Result<Target, ImageFetchError> {
    let host = url
        .host_str()
        .ok_or(ImageFetchError::TargetBlocked)?
        .trim_matches(['[', ']']);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if public_ip(ip) {
            Ok(Target::Pinned(vec![ip]))
        } else {
            Err(ImageFetchError::TargetBlocked)
        };
    }
    if local_hostname(host) {
        return Err(ImageFetchError::TargetBlocked);
    }
    if route.proxy.is_some() {
        if !route.image_proxy_dns {
            return Err(ImageFetchError::ProxyPolicyRequired);
        }
        if url.scheme() != "https" {
            return Err(ImageFetchError::TargetBlocked);
        }
        // The user selected this proxy as the resolver. A local lookup would
        // both leak the target and say nothing about the proxy's final address.
        return Ok(Target::ProxyDomain);
    }
    let port = url
        .port_or_known_default()
        .ok_or(ImageFetchError::TargetBlocked)?;
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| ImageFetchError::Dns)?
        .take(MAX_RESOLVED_ADDRESSES)
        .map(|address| address.ip())
        .collect();
    Ok(Target::Pinned(resolved_targets(url, &addresses)?))
}

fn fake_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.octets()[0] == 198 && matches!(ip.octets()[1], 18 | 19),
        IpAddr::V6(ip) => ip.segments()[..3] == [0x2001, 2, 0],
    }
}

pub(super) fn resolved_targets(
    url: &Url,
    addresses: &[IpAddr],
) -> Result<Vec<IpAddr>, ImageFetchError> {
    let tls_domain = url.scheme() == "https" && matches!(url.host(), Some(url::Host::Domain(_)));
    let mut candidates = Vec::new();
    for &ip in addresses.iter().take(MAX_RESOLVED_ADDRESSES) {
        // The known TUN mapping exception requires a named HTTPS origin. TLS
        // authenticates that origin, not the mapping's ultimate public address.
        if (public_ip(ip) || (tls_domain && fake_ip(ip))) && !candidates.contains(&ip) {
            candidates.push(ip);
            if candidates.len() == MAX_CANDIDATES {
                break;
            }
        }
    }
    if candidates.is_empty() {
        Err(ImageFetchError::TargetBlocked)
    } else {
        Ok(candidates)
    }
}

#[derive(Clone)]
struct PinnedResolver {
    host: String,
    addresses: Vec<SocketAddr>,
}

impl Service<Name> for PinnedResolver {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = io::Error;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        // This resolver has no OS/network resolver and cannot follow another
        // hostname. Hyper handles address-family racing within this snapshot.
        ready(if name.as_str() == self.host {
            Ok(self.addresses.clone().into_iter())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unbound image host",
            ))
        })
    }
}

pub(super) async fn connect_target(
    route: &NetRoute,
    url: &Url,
    target: Target,
) -> Result<BoxedStream, ImageFetchError> {
    let host = url
        .host_str()
        .ok_or(ImageFetchError::TargetBlocked)?
        .trim_matches(['[', ']']);
    let port = url
        .port_or_known_default()
        .ok_or(ImageFetchError::TargetBlocked)?;
    match target {
        Target::ProxyDomain => crate::net::connect(route, host, port)
            .await
            .map_err(network_error),
        Target::Pinned(addresses) if route.proxy.is_some() => {
            // Only an already checked public literal enters this branch.
            let ip = addresses.first().ok_or(ImageFetchError::TargetBlocked)?;
            crate::net::connect(route, &ip.to_string(), port)
                .await
                .map_err(network_error)
        }
        Target::Pinned(addresses) => connect_pinned(url, addresses).await,
    }
}

async fn connect_pinned(url: &Url, addresses: Vec<IpAddr>) -> Result<BoxedStream, ImageFetchError> {
    let resolver = PinnedResolver {
        host: url
            .host_str()
            .ok_or(ImageFetchError::TargetBlocked)?
            .to_owned(),
        addresses: addresses
            .into_iter()
            .map(|ip| SocketAddr::new(ip, 0))
            .collect(),
    };
    let mut connector = HttpConnector::new_with_resolver(resolver);
    connector.enforce_http(false);
    connector.set_connect_timeout(Some(crate::net::CONNECT_TIMEOUT));
    connector.set_happy_eyeballs_timeout(Some(HAPPY_EYEBALLS_DELAY));
    let uri = url
        .as_str()
        .parse()
        .map_err(|_| ImageFetchError::TargetBlocked)?;
    let stream = connector
        .call(uri)
        .await
        .map_err(|_| ImageFetchError::Connect)?;
    Ok(Box::new(stream.into_inner()))
}

pub(super) fn network_error(error: NetError) -> ImageFetchError {
    match error {
        NetError::Connect(_) => ImageFetchError::Connect,
        NetError::Proxy(_) => ImageFetchError::Proxy,
        NetError::Tls(_) => ImageFetchError::Tls,
        NetError::Timeout => ImageFetchError::Timeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::types::ProxyKind;
    use crate::net::test_support;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[tokio::test]
    async fn pinned_connector_uses_the_snapshot_and_falls_back_within_address_family() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "https://not-in-dns.invalid:{}/image",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.write_all(b"pinned").await.unwrap();
        });
        // Bypass only target policy in this connector test: both test addresses
        // are local, while production passes exclusively validated candidates.
        let mut stream = tokio::time::timeout(
            Duration::from_secs(5),
            connect_pinned(
                &url,
                vec!["127.0.0.2".parse().unwrap(), "127.0.0.1".parse().unwrap()],
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"pinned");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pinned_connector_falls_back_between_address_families() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "https://dual-stack.invalid:{}/image",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let connected = tokio::time::timeout(
            Duration::from_secs(5),
            connect_pinned(
                &url,
                vec!["::1".parse().unwrap(), "127.0.0.1".parse().unwrap()],
            ),
        )
        .await
        .unwrap();
        assert!(connected.is_ok());
        assert!(listener.accept().await.is_ok());
    }

    #[tokio::test]
    async fn trusted_proxies_receive_the_unresolved_domain_and_tls_checks_original_name() {
        for (kind, valid_name) in [
            (ProxyKind::Socks5, true),
            (ProxyKind::HttpConnect, true),
            (ProxyKind::Socks5, false),
            (ProxyKind::HttpConnect, false),
        ] {
            let certificate_name = if valid_name {
                "images.example.invalid"
            } else {
                "wrong.example.invalid"
            };
            let tls = test_support::TestTls::for_names(&[certificate_name]);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream = listener.local_addr().unwrap().port();
            let acceptor = tokio_rustls::TlsAcceptor::from(tls.server);
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                if let Ok(mut stream) = acceptor.accept(stream).await {
                    let _ = stream.write_all(b"tls").await;
                }
            });
            let (port, record) = match kind {
                ProxyKind::Socks5 => test_support::fake_socks5(upstream).await,
                ProxyKind::HttpConnect => test_support::fake_http_connect(upstream).await,
            };
            let mut route = test_support::proxy_route(kind, port);
            route.image_proxy_dns = true;
            let url = Url::parse("https://images.example.invalid/image").unwrap();
            let checked = target(&route, &url).await.unwrap();
            assert_eq!(checked, Target::ProxyDomain);
            let stream = connect_target(&route, &url, checked).await.unwrap();
            let upgraded =
                crate::net::upgrade_tls_with(stream, "images.example.invalid", tls.client)
                    .await
                    .map_err(network_error);
            if valid_name {
                let mut stream = upgraded.unwrap();
                let mut bytes = [0; 3];
                stream.read_exact(&mut bytes).await.unwrap();
                assert_eq!(&bytes, b"tls");
            } else {
                assert!(matches!(upgraded, Err(ImageFetchError::Tls)));
            }
            assert!(record.lock().unwrap()[0].contains("images.example.invalid:443"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn proxy_policy_blocks_plaintext_local_names_and_never_falls_back() {
        let mut route =
            test_support::proxy_route(ProxyKind::HttpConnect, test_support::dead_port().await);
        let https = Url::parse("https://unresolvable.example.invalid/image").unwrap();
        assert_eq!(
            target(&route, &https).await.unwrap_err(),
            ImageFetchError::ProxyPolicyRequired
        );
        route.image_proxy_dns = true;
        for value in [
            "http://images.example.invalid/a",
            "https://localhost/a",
            "https://thing.local/a",
            "https://local./a",
            "https://metadata.google.internal/a",
            "https://127.0.0.1/a",
            "https://[::1]/a",
        ] {
            assert_eq!(
                target(&route, &Url::parse(value).unwrap())
                    .await
                    .unwrap_err(),
                ImageFetchError::TargetBlocked
            );
        }
        let checked = target(&route, &https).await.unwrap();
        assert!(matches!(
            connect_target(&route, &https, checked).await,
            Err(ImageFetchError::Proxy)
        ));
    }
}
