//! Proxy tunnels. The target host name is always sent to the proxy unresolved,
//! so DNS happens on the proxy side and nothing leaks to the local resolver.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;

use super::{BoxedStream, NetError, ProxyRoute};
use crate::accounts::types::ProxyKind;

/// Upper bound for the proxy's CONNECT response headers.
const MAX_HTTP_RESPONSE: usize = 8 * 1024;

pub(super) async fn connect(
    proxy: &ProxyRoute,
    host: &str,
    port: u16,
) -> Result<BoxedStream, NetError> {
    match proxy.kind {
        ProxyKind::Socks5 => socks5(proxy, host, port).await,
        ProxyKind::HttpConnect => http_connect(proxy, host, port).await,
    }
}

async fn socks5(proxy: &ProxyRoute, host: &str, port: u16) -> Result<BoxedStream, NetError> {
    let proxy_address = (proxy.host.as_str(), proxy.port);
    let target = (host, port);
    let stream = match &proxy.auth {
        Some((username, password)) => {
            Socks5Stream::connect_with_password(
                proxy_address,
                target,
                username,
                password.expose_secret(),
            )
            .await
        }
        None => Socks5Stream::connect(proxy_address, target).await,
    }
    .map_err(|error| NetError::Proxy(error.to_string()))?;
    Ok(Box::new(stream))
}

async fn http_connect(proxy: &ProxyRoute, host: &str, port: u16) -> Result<BoxedStream, NetError> {
    let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port))
        .await
        .map_err(|error| NetError::Proxy(error.to_string()))?;
    stream
        .write_all(connect_request(proxy, host, port).as_bytes())
        .await
        .map_err(|error| NetError::Proxy(error.to_string()))?;

    // Read byte by byte so nothing after the header terminator is consumed;
    // those bytes already belong to the tunnelled protocol.
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= MAX_HTTP_RESPONSE {
            return Err(NetError::Proxy("proxy response headers too large".into()));
        }
        let byte = stream
            .read_u8()
            .await
            .map_err(|error| NetError::Proxy(error.to_string()))?;
        response.push(byte);
    }
    check_connect_response(&response)?;
    Ok(Box::new(stream))
}

fn connect_request(proxy: &ProxyRoute, host: &str, port: u16) -> String {
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some((username, password)) = &proxy.auth {
        let credentials = STANDARD.encode(format!("{username}:{}", password.expose_secret()));
        request.push_str(&format!("Proxy-Authorization: Basic {credentials}\r\n"));
    }
    request.push_str("\r\n");
    request
}

fn check_connect_response(response: &[u8]) -> Result<(), NetError> {
    let status_line = response
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).trim().to_owned())
        .unwrap_or_default();
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or_default();
    let code = parts.next().unwrap_or_default();
    if version.starts_with("HTTP/1.") && code == "200" {
        Ok(())
    } else if code == "407" {
        Err(NetError::Proxy(
            "proxy authentication required or rejected".into(),
        ))
    } else {
        Err(NetError::Proxy(format!(
            "proxy refused the tunnel: {status_line}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;
    use crate::secrets::SecretValue;

    fn route(port: u16, auth: bool) -> ProxyRoute {
        ProxyRoute {
            kind: ProxyKind::HttpConnect,
            host: "127.0.0.1".into(),
            port,
            auth: auth.then(|| ("alice".into(), SecretValue::new("s3cret".into()))),
        }
    }

    #[test]
    fn connect_request_asks_the_proxy_to_resolve_the_target() {
        let request = connect_request(&route(8080, true), "imap.example.com", 993);
        assert!(request.starts_with(
            "CONNECT imap.example.com:993 HTTP/1.1\r\nHost: imap.example.com:993\r\n"
        ));
        assert!(request.contains("Proxy-Authorization: Basic YWxpY2U6czNjcmV0\r\n"));
        assert!(request.ends_with("\r\n\r\n"));
        assert!(connect_request(&route(8080, false), "::1", 993).starts_with("CONNECT [::1]:993 "));
    }

    #[test]
    fn only_a_200_response_opens_the_tunnel() {
        assert!(check_connect_response(b"HTTP/1.1 200 Connection established\r\n\r\n").is_ok());
        assert!(check_connect_response(b"HTTP/1.0 200 OK\r\n\r\n").is_ok());
        assert!(matches!(
            check_connect_response(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n"),
            Err(NetError::Proxy(reason)) if reason.contains("authentication")
        ));
        assert!(check_connect_response(b"HTTP/1.1 403 Forbidden\r\n\r\n").is_err());
        assert!(check_connect_response(b"SSH-2.0-OpenSSH\r\n\r\n").is_err());
    }

    #[tokio::test]
    async fn tunnel_keeps_bytes_sent_right_after_the_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            let mut line = String::new();
            socket.read_line(&mut line).await.unwrap();
            assert_eq!(line, "CONNECT imap.example.com:143 HTTP/1.1\r\n");
            while line != "\r\n" {
                line.clear();
                socket.read_line(&mut line).await.unwrap();
            }
            // The greeting arrives in the same write as the response headers.
            socket
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\n\r\n* OK IMAP ready\r\n")
                .await
                .unwrap();
        });

        let mut stream = connect(&route(port, false), "imap.example.com", 143)
            .await
            .unwrap();
        let mut greeting = String::new();
        stream.read_to_string(&mut greeting).await.unwrap();
        assert_eq!(greeting, "* OK IMAP ready\r\n");
        proxy.await.unwrap();
    }

    #[tokio::test]
    async fn refused_proxy_is_an_error_not_a_direct_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(matches!(
            connect(&route(port, false), "imap.example.com", 143).await,
            Err(NetError::Proxy(_))
        ));
    }
}
