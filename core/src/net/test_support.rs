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
        image_proxy_dns: false,
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

/// One request as the mock HTTP server saw it.
#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub method: String,
    /// Path and query.
    pub target: String,
    /// Lowercase names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub(crate) struct MockReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MockReply {
    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

type Responder = Arc<dyn Fn(&Recorded) -> MockReply + Send + Sync>;

/// A plain-HTTP server on 127.0.0.1 that answers every request through
/// `responder` and records what it received. One request per connection.
pub(crate) struct MockHttp {
    pub port: u16,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockHttp {
    pub(crate) fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub(crate) fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub(crate) fn last(&self) -> Recorded {
        self.requests.lock().unwrap().last().cloned().unwrap()
    }
}

pub(crate) async fn mock_http(
    responder: impl Fn(&Recorded) -> MockReply + Send + Sync + 'static,
) -> MockHttp {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let responder: Responder = Arc::new(responder);
    let record = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (record, responder) = (record.clone(), responder.clone());
            tokio::spawn(async move {
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_owned();
                let target = parts.next().unwrap_or_default().to_owned();
                let mut headers = Vec::new();
                loop {
                    line.clear();
                    stream.read_line(&mut line).await.unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.trim_end().split_once(':') {
                        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
                    }
                }
                let length = headers
                    .iter()
                    .find(|(name, _)| name == "content-length")
                    .and_then(|(_, value)| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0u8; length];
                stream.read_exact(&mut body).await.unwrap();
                let request = Recorded {
                    method,
                    target,
                    headers,
                    body,
                };
                let reply = responder(&request);
                record.lock().unwrap().push(request);
                let mut head = format!(
                    "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.body.len()
                );
                for (name, value) in &reply.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");
                let stream = stream.get_mut();
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&reply.body).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    MockHttp { port, requests }
}
