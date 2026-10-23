//! Feasibility probe: can `lettre` run SMTP over MailX's own account-routed streams?
//!
//! `lettre` only ever sees a stream opened by `net::connect`, so the account
//! route (direct, SOCKS5, HTTP CONNECT) and the no-fallback rule stay in `net`.
//! This module is test-only and does not ship any SMTP sending code.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use lettre::Address;
use lettre::address::Envelope;
use lettre::transport::smtp::client::{AsyncSmtpConnection, AsyncTokioStream};
use lettre::transport::smtp::extension::ClientId;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf,
};
use tokio::net::{TcpListener, TcpStream};

use super::{BoxedStream, NetError, NetRoute, ProxyRoute, connect};
use crate::accounts::types::ProxyKind;

/// `lettre` requires `Sync`; `BoxedStream` is only `Send`. Access is serialized by a mutex.
#[derive(Debug)]
struct SyncStream(Mutex<BoxedStream>);

impl SyncStream {
    fn boxed(stream: BoxedStream) -> Box<dyn AsyncTokioStream> {
        Box::new(Self(Mutex::new(stream)))
    }

    fn with<R>(&self, f: impl FnOnce(Pin<&mut BoxedStream>) -> R) -> R {
        let mut guard = self.0.lock().unwrap();
        f(Pin::new(&mut *guard))
    }
}

impl AsyncRead for SyncStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.with(|stream| stream.poll_read(cx, buf))
    }
}

impl AsyncWrite for SyncStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.with(|stream| stream.poll_write(cx, buf))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.with(|stream| stream.poll_flush(cx))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.with(|stream| stream.poll_shutdown(cx))
    }
}

impl AsyncTokioStream for SyncStream {
    fn peer_addr(&self) -> io::Result<SocketAddr> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "routed stream"))
    }
}

/// What the fake server observed.
#[derive(Default, Debug)]
struct Seen {
    commands: Vec<String>,
    data: String,
}

/// A one-session plain-text SMTP server.
async fn fake_smtp() -> (u16, Arc<Mutex<Seen>>) {
    fake_smtp_with(None).await
}

/// `starttls` offers STARTTLS and upgrades with the given server config.
async fn fake_smtp_with(starttls: Option<Arc<rustls::ServerConfig>>) -> (u16, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let record = seen.clone();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        serve_smtp(Box::new(socket), record, starttls).await;
    });
    (port, seen)
}

trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

async fn serve_smtp(
    socket: Box<dyn Duplex>,
    seen: Arc<Mutex<Seen>>,
    mut starttls: Option<Arc<rustls::ServerConfig>>,
) {
    let mut socket = BufReader::new(socket);
    socket
        .get_mut()
        .write_all(b"220 fake ESMTP\r\n")
        .await
        .unwrap();
    let mut line = String::new();
    loop {
        line.clear();
        if socket.read_line(&mut line).await.unwrap() == 0 {
            return;
        }
        let command = line.trim_end().to_owned();
        seen.lock().unwrap().commands.push(command.clone());
        let upper = command.to_ascii_uppercase();
        let reply: &[u8] = if upper.starts_with("EHLO") {
            if starttls.is_some() {
                b"250-fake\r\n250-STARTTLS\r\n250 8BITMIME\r\n"
            } else {
                b"250-fake\r\n250 8BITMIME\r\n"
            }
        } else if upper.starts_with("STARTTLS") {
            socket.get_mut().write_all(b"220 ready\r\n").await.unwrap();
            let config = starttls.take().unwrap();
            let plain = socket.into_inner();
            let tls = tokio_rustls::TlsAcceptor::from(config)
                .accept(plain)
                .await
                .unwrap();
            socket = BufReader::new(Box::new(tls));
            continue;
        } else if upper.starts_with("DATA") {
            socket.get_mut().write_all(b"354 go\r\n").await.unwrap();
            let mut body = String::new();
            loop {
                line.clear();
                socket.read_line(&mut line).await.unwrap();
                if line == ".\r\n" {
                    break;
                }
                body.push_str(&line);
            }
            seen.lock().unwrap().data = body;
            b"250 queued\r\n"
        } else if upper.starts_with("QUIT") {
            socket.get_mut().write_all(b"221 bye\r\n").await.unwrap();
            return;
        } else {
            b"250 ok\r\n"
        };
        socket.get_mut().write_all(reply).await.unwrap();
    }
}

/// A SOCKS5 proxy (no auth) that records the requested target host and forwards to `upstream`.
async fn fake_socks5(upstream: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
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
async fn fake_http_connect(upstream: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
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

fn proxy(kind: ProxyKind, port: u16) -> NetRoute {
    NetRoute {
        proxy: Some(ProxyRoute {
            kind,
            host: "127.0.0.1".into(),
            port,
            auth: None,
        }),
    }
}

/// Sends one message over `stream` with `lettre` and returns the server reply code.
async fn send_over(stream: BoxedStream) -> String {
    let mut connection = AsyncSmtpConnection::connect_with_transport(
        SyncStream::boxed(stream),
        &ClientId::Domain("mailx.test".into()),
    )
    .await
    .unwrap();
    let envelope = Envelope::new(
        Some("me@example.test".parse::<Address>().unwrap()),
        vec!["you@example.test".parse::<Address>().unwrap()],
    )
    .unwrap();
    let reply = connection
        .send(&envelope, b"Subject: probe\r\n\r\nhello probe\r\n")
        .await
        .unwrap();
    connection.quit().await.unwrap();
    reply.code().to_string()
}

fn assert_delivered(seen: &Arc<Mutex<Seen>>) {
    let seen = seen.lock().unwrap();
    assert!(seen.data.contains("hello probe"), "server saw: {seen:?}");
    assert!(
        seen.commands
            .iter()
            .any(|c| c.starts_with("MAIL FROM:<me@example.test>"))
    );
    assert!(
        seen.commands
            .iter()
            .any(|c| c.starts_with("RCPT TO:<you@example.test>"))
    );
}

#[tokio::test]
async fn direct_route_carries_smtp() {
    let (port, seen) = fake_smtp().await;
    let stream = connect(&NetRoute::default(), "127.0.0.1", port)
        .await
        .unwrap();
    assert_eq!(send_over(stream).await, "250");
    assert_delivered(&seen);
}

#[tokio::test]
async fn socks5_route_carries_smtp_with_unresolved_host() {
    let (smtp_port, seen) = fake_smtp().await;
    let (proxy_port, targets) = fake_socks5(smtp_port).await;
    let stream = connect(
        &proxy(ProxyKind::Socks5, proxy_port),
        "smtp.example.test",
        587,
    )
    .await
    .unwrap();
    assert_eq!(send_over(stream).await, "250");
    assert_delivered(&seen);
    assert_eq!(*targets.lock().unwrap(), ["smtp.example.test:587"]);
}

#[tokio::test]
async fn http_connect_route_carries_smtp_with_unresolved_host() {
    let (smtp_port, seen) = fake_smtp().await;
    let (proxy_port, requests) = fake_http_connect(smtp_port).await;
    let stream = connect(
        &proxy(ProxyKind::HttpConnect, proxy_port),
        "smtp.example.test",
        465,
    )
    .await
    .unwrap();
    assert_eq!(send_over(stream).await, "250");
    assert_delivered(&seen);
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0], "CONNECT smtp.example.test:465 HTTP/1.1");
}

#[tokio::test]
async fn dead_proxy_never_falls_back_to_a_direct_smtp_connection() {
    // A reachable "direct" server exists; the proxy does not.
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let direct_port = direct.local_addr().unwrap().port();
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);

    for kind in [ProxyKind::Socks5, ProxyKind::HttpConnect] {
        let result = connect(&proxy(kind, dead_port), "127.0.0.1", direct_port).await;
        assert!(
            matches!(result, Err(NetError::Proxy(_))),
            "{kind:?}: {result:?}"
        );
    }
    let accepted = tokio::time::timeout(Duration::from_millis(300), direct.accept()).await;
    assert!(accepted.is_err(), "a direct connection was attempted");
}

/// A self-signed certificate for `localhost`: (server config, client trust store, PEM).
fn test_tls() -> (Arc<rustls::ServerConfig>, Arc<rustls::ClientConfig>, String) {
    let key = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let pem = key.cert.pem();
    let cert = key.cert.der().clone();
    let server = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.clone()],
            rustls::pki_types::PrivateKeyDer::try_from(key.signing_key.serialize_der()).unwrap(),
        )
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert).unwrap();
    let client = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    (Arc::new(server), Arc::new(client), pem)
}

#[tokio::test]
async fn starttls_upgrades_on_a_socks5_routed_stream() {
    use lettre::transport::smtp::client::{Certificate, TlsParameters};

    let (server_config, _, pem) = test_tls();
    let (smtp_port, seen) = fake_smtp_with(Some(server_config)).await;
    let (proxy_port, targets) = fake_socks5(smtp_port).await;
    let stream = connect(&proxy(ProxyKind::Socks5, proxy_port), "localhost", 587)
        .await
        .unwrap();

    let hello = ClientId::Domain("mailx.test".into());
    let mut connection =
        AsyncSmtpConnection::connect_with_transport(SyncStream::boxed(stream), &hello)
            .await
            .unwrap();
    assert!(connection.can_starttls());
    let tls = TlsParameters::builder("localhost".into())
        .add_root_certificate(Certificate::from_pem(pem.as_bytes()).unwrap())
        .build_rustls()
        .unwrap();
    connection.starttls(tls, &hello).await.unwrap();
    assert!(connection.is_encrypted());
    let envelope = Envelope::new(
        Some("me@example.test".parse::<Address>().unwrap()),
        vec!["you@example.test".parse::<Address>().unwrap()],
    )
    .unwrap();
    connection
        .send(&envelope, b"Subject: probe\r\n\r\nhello probe\r\n")
        .await
        .unwrap();
    connection.quit().await.unwrap();
    assert_delivered(&seen);
    assert_eq!(*targets.lock().unwrap(), ["localhost:587"]);
}

#[tokio::test]
async fn implicit_tls_over_a_routed_stream_carries_smtp() {
    let (server_config, client_config, _) = test_tls();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let smtp_port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let record = seen.clone();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(server_config)
            .accept(socket)
            .await
            .unwrap();
        serve_smtp(Box::new(tls), record, None).await;
    });
    let (proxy_port, _) = fake_socks5(smtp_port).await;
    let stream = connect(&proxy(ProxyKind::Socks5, proxy_port), "localhost", 465)
        .await
        .unwrap();
    let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let tls = tokio_rustls::TlsConnector::from(client_config)
        .connect(name, stream)
        .await
        .unwrap();
    assert_eq!(send_over(Box::new(tls)).await, "250");
    assert_delivered(&seen);
}
