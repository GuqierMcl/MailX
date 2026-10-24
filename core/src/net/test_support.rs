//! In-process fakes for network tests: SOCKS5 and HTTP CONNECT proxies that
//! forward to a local upstream, and a throwaway TLS identity. Everything
//! listens on 127.0.0.1; nothing here talks to a real server.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use super::{NetRoute, ProxyRoute};
use crate::accounts::types::ProxyKind;

/// A SOCKS5 proxy (no auth) that records the requested target host and forwards to `upstream`.
pub(crate) async fn fake_socks5(upstream: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let targets = Arc::new(Mutex::new(Vec::new()));
    let record = targets.clone();
    tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        let mut head = [0u8; 2];
        client.read_exact(&mut head).await.unwrap();
        let mut methods = vec![0u8; head[1] as usize];
        client.read_exact(&mut methods).await.unwrap();
        client.write_all(&[5, 0]).await.unwrap();
        let mut request = [0u8; 4];
        client.read_exact(&mut request).await.unwrap();
        assert_eq!(
            request[3], 3,
            "target must be sent as an unresolved domain name"
        );
        let mut len = [0u8; 1];
        client.read_exact(&mut len).await.unwrap();
        let mut host = vec![0u8; len[0] as usize];
        client.read_exact(&mut host).await.unwrap();
        let mut target_port = [0u8; 2];
        client.read_exact(&mut target_port).await.unwrap();
        record.lock().unwrap().push(format!(
            "{}:{}",
            String::from_utf8(host).unwrap(),
            u16::from_be_bytes(target_port)
        ));
        let mut server = TcpStream::connect(("127.0.0.1", upstream)).await.unwrap();
        client
            .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
        let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
    });
    (port, targets)
}

/// An HTTP CONNECT proxy that records the request line and forwards to `upstream`.
pub(crate) async fn fake_http_connect(upstream: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let record = requests.clone();
    tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let mut client = BufReader::new(client);
        let mut line = String::new();
        client.read_line(&mut line).await.unwrap();
        record.lock().unwrap().push(line.trim_end().to_owned());
        while line != "\r\n" {
            line.clear();
            client.read_line(&mut line).await.unwrap();
        }
        let mut server = TcpStream::connect(("127.0.0.1", upstream)).await.unwrap();
        client
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
            .await
            .unwrap();
        let _ = tokio::io::copy_bidirectional(client.get_mut(), &mut server).await;
    });
    (port, requests)
}

/// A route through a local proxy on `port`.
pub(crate) fn proxy_route(kind: ProxyKind, port: u16) -> NetRoute {
    NetRoute {
        proxy: Some(ProxyRoute {
            kind,
            host: "127.0.0.1".into(),
            port,
            auth: None,
        }),
    }
}

/// A local port nothing listens on.
pub(crate) async fn dead_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// A self-signed identity for `names`. The client config trusts exactly that
/// certificate, so a handshake succeeds only for the listed names.
pub(crate) struct TestTls {
    pub server: Arc<rustls::ServerConfig>,
    pub client: Arc<rustls::ClientConfig>,
}

impl TestTls {
    pub(crate) fn for_names(names: &[&str]) -> Self {
        let key = rcgen::generate_simple_self_signed(
            names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>(),
        )
        .unwrap();
        let cert = key.cert.der().clone();
        let server = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.clone()],
                rustls::pki_types::PrivateKeyDer::try_from(key.signing_key.serialize_der())
                    .unwrap(),
            )
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert).unwrap();
        let client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Self {
            server: Arc::new(server),
            client: Arc::new(client),
        }
    }
}
