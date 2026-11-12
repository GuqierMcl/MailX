//! One HTTP/1.1 connection per account, because NTLM is bound to it.
//!
//! `Type 3` must travel on the same TCP connection as `Type 1` and the server's
//! challenge (MS-NLMP 3.1.5.1), so this client deliberately has no pool: it
//! wraps a single stream, keeps one `hyper` HTTP/1.1 sender on it, and every
//! request goes through that sender in order. A dropped connection is an error,
//! never something to paper over by opening another one.

use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST, WWW_AUTHENTICATE};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::time::timeout;

use crate::net::{self, BoxedStream, NetError, NetRoute};

use super::ntlm::{self, Credentials};

/// One request, from the first byte to the last of the response body.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// A SOAP response larger than this is refused instead of parsed
/// (`docs/architecture/ews-backend.md` §3).
pub(crate) const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
/// What authentication asks for. Its content is irrelevant: any authenticated
/// request proves the credentials, and an unauthenticated one only produces the
/// `401` this client needs.
const PROBE_BODY: &str = concat!(
    r#"<?xml version="1.0" encoding="utf-8"?>"#,
    r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">"#,
    r#"<soap:Header><t:RequestServerVersion xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types" Version="Exchange2013"/></soap:Header>"#,
    r#"<soap:Body><GetFolder xmlns="http://schemas.microsoft.com/exchange/services/2006/messages">"#,
    r#"<FolderShape><BaseShape>IdOnly</BaseShape></FolderShape>"#,
    r#"<FolderIds><DistinguishedFolderId Id="inbox"/></FolderIds>"#,
    r#"</GetFolder></soap:Body></soap:Envelope>"#,
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HttpError {
    Net(NetError),
    Timeout,
    /// `Type 3` was answered with `401`/`403`: the credentials or the domain are wrong.
    Rejected,
    /// The server never offered NTLM, so only a scheme we do not implement is left.
    NoNtlm,
    /// The server's answer is not something this client will parse.
    Malformed,
    /// The response body went past [`MAX_RESPONSE_BYTES`].
    TooLarge,
    /// The connection failed mid-request.
    Transport,
}

/// An authenticated, connection-bound EWS endpoint.
pub(crate) struct EwsHttp {
    sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
    host: String,
    path: String,
}

impl EwsHttp {
    /// Connects over TLS along the account's route, then authenticates.
    pub(crate) async fn open(
        route: &NetRoute,
        host: &str,
        port: u16,
        path: &str,
        credentials: &Credentials<'_>,
    ) -> Result<Self, HttpError> {
        let stream = net::connect(route, host, port)
            .await
            .map_err(HttpError::Net)?;
        let stream = net::upgrade_tls(stream, host)
            .await
            .map_err(HttpError::Net)?;
        Self::with_stream(stream, host, path, credentials).await
    }

    /// The same, over a stream the caller already established. TLS is the
    /// caller's job, which keeps the tests on plain TCP.
    pub(crate) async fn with_stream(
        stream: BoxedStream,
        host: &str,
        path: &str,
        credentials: &Credentials<'_>,
    ) -> Result<Self, HttpError> {
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|_| HttpError::Transport)?;
        // The connection task ends when the sender is dropped.
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut http = Self {
            sender,
            host: host.to_owned(),
            path: path.to_owned(),
        };
        http.authenticate(credentials).await?;
        Ok(http)
    }

    /// Runs the three-message handshake on this connection. Every response is
    /// drained first: `hyper` cannot send the next request while a body is
    /// still unread, and the whole point is to keep one connection.
    async fn authenticate(&mut self, credentials: &Credentials<'_>) -> Result<(), HttpError> {
        let response = self.send(None).await?;
        let status = response.status();
        let offered = if status.is_success() {
            None
        } else {
            Some(challenge_token(response.headers())?)
        };
        drain(response).await?;
        if status.is_success() {
            // The endpoint answered an anonymous request. That is not a failure,
            // but the caller's first real request decides what it means.
            return Ok(());
        }
        // The first refusal names `NTLM` without a challenge payload.
        if offered.flatten().is_some() {
            return Err(HttpError::Malformed);
        }
        let type1 = ntlm::negotiate();
        let response = self.send(Some(&type1)).await?;
        let status = response.status();
        let token = challenge_token(response.headers())?;
        drain(response).await?;
        if status.is_success() {
            return Err(HttpError::Malformed);
        }
        let token = token.ok_or(HttpError::NoNtlm)?;
        let challenge = ntlm::parse_challenge(&token).map_err(|_| HttpError::NoNtlm)?;
        let type3 = ntlm::authenticate(
            credentials,
            &challenge,
            ntlm::filetime(std::time::SystemTime::now()),
            ntlm::client_challenge(),
            None,
        );
        let response = self.send(Some(&type3)).await?;
        let status = response.status();
        drain(response).await?;
        if status.is_success() {
            return Ok(());
        }
        Err(match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => HttpError::Rejected,
            _ => HttpError::Transport,
        })
    }

    /// Posts one SOAP request on the authenticated connection.
    pub(crate) async fn post(&mut self, action: &str, body: &str) -> Result<String, HttpError> {
        let response = self.send_with(None, Some(action), body.to_owned()).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            // The connection was re-authenticated by someone else, or the
            // session expired: the caller has to start a new connection.
            return Err(HttpError::Rejected);
        }
        if !response.status().is_success() {
            return Err(HttpError::Transport);
        }
        read_body(response).await
    }

    async fn send(
        &mut self,
        authorization: Option<&[u8]>,
    ) -> Result<Response<Incoming>, HttpError> {
        self.send_with(authorization, None, PROBE_BODY.to_owned())
            .await
    }

    async fn send_with(
        &mut self,
        authorization: Option<&[u8]>,
        action: Option<&str>,
        body: String,
    ) -> Result<Response<Incoming>, HttpError> {
        // The connection task has to run before the sender accepts a request.
        if timeout(REQUEST_TIMEOUT, self.sender.ready())
            .await
            .map_err(|_| HttpError::Timeout)?
            .is_err()
        {
            return Err(HttpError::Transport);
        }
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(self.path.as_str())
            .header(HOST, &self.host)
            .header(CONTENT_TYPE, "text/xml; charset=utf-8")
            .header(CONTENT_LENGTH, body.len());
        if let Some(action) = action {
            request = request.header(
                "SOAPAction",
                format!(
                    "\"http://schemas.microsoft.com/exchange/services/2006/messages/{action}\""
                ),
            );
        }
        if let Some(authorization) = authorization {
            let token = base64::engine::general_purpose::STANDARD.encode(authorization);
            request = request.header(AUTHORIZATION, format!("NTLM {token}"));
        }
        let request = request
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| HttpError::Malformed)?;
        timeout(REQUEST_TIMEOUT, self.sender.send_request(request))
            .await
            .map_err(|_| HttpError::Timeout)?
            .map_err(|_| HttpError::Transport)
    }
}

/// Reads the base64 challenge out of `WWW-Authenticate`, if the server offered
/// `NTLM` at all. `Ok(None)` means "NTLM offered, no payload yet".
fn challenge_token(headers: &hyper::HeaderMap) -> Result<Option<Vec<u8>>, HttpError> {
    let mut offered = false;
    for value in headers.get_all(WWW_AUTHENTICATE) {
        let text = value.to_str().map_err(|_| HttpError::Malformed)?;
        // One header may carry several challenges, comma separated. Base64 never
        // contains a comma, so splitting on it cannot cut a challenge in half.
        for part in text.split(',') {
            let part = part.trim();
            let Some(scheme) = part.get(..4) else {
                continue;
            };
            if !scheme.eq_ignore_ascii_case("ntlm") {
                continue;
            }
            offered = true;
            let token = part[4..].trim();
            if token.is_empty() {
                continue;
            }
            return base64::engine::general_purpose::STANDARD
                .decode(token)
                .map(Some)
                .map_err(|_| HttpError::Malformed);
        }
    }
    if offered {
        Ok(None)
    } else {
        Err(HttpError::NoNtlm)
    }
}

/// Reads and discards a body so the connection can carry another request.
async fn drain(response: Response<Incoming>) -> Result<(), HttpError> {
    let body = Limited::new(response.into_body(), MAX_RESPONSE_BYTES);
    timeout(REQUEST_TIMEOUT, body.collect())
        .await
        .map_err(|_| HttpError::Timeout)?
        .map_err(|_| HttpError::Transport)?;
    Ok(())
}

async fn read_body(response: Response<Incoming>) -> Result<String, HttpError> {
    let body = Limited::new(response.into_body(), MAX_RESPONSE_BYTES);
    let collected = timeout(REQUEST_TIMEOUT, body.collect())
        .await
        .map_err(|_| HttpError::Timeout)?
        .map_err(|_| HttpError::TooLarge)?;
    String::from_utf8(collected.to_bytes().to_vec()).map_err(|_| HttpError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use crate::ews::ntlm::NEGOTIATE_FLAGS;
    use crate::secrets::SecretValue;

    /// A `Type 2` message shaped like the ones Exchange sends: 48-byte header,
    /// target info at its end.
    fn type2() -> Vec<u8> {
        let target_info = [0x02u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let mut message = Vec::new();
        message.extend_from_slice(b"NTLMSSP\0");
        message.extend_from_slice(&2u32.to_le_bytes());
        message.extend_from_slice(&[0u8; 8]);
        message.extend_from_slice(&NEGOTIATE_FLAGS.to_le_bytes());
        message.extend_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        message.extend_from_slice(&[0u8; 8]);
        message.extend_from_slice(&(target_info.len() as u16).to_le_bytes());
        message.extend_from_slice(&(target_info.len() as u16).to_le_bytes());
        message.extend_from_slice(&48u32.to_le_bytes());
        message.extend_from_slice(&target_info);
        message
    }

    /// Reads one request off the wire: headers, then the body `Content-Length`
    /// announces.
    async fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            let read = stream.read(&mut chunk).await.expect("read");
            assert!(read > 0, "client closed before sending a request");
            buffer.extend_from_slice(&chunk[..read]);
            let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let head = String::from_utf8_lossy(&buffer[..end]).to_string();
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if buffer.len() >= end + 4 + length {
                return (head, buffer[end + 4..end + 4 + length].to_vec());
            }
        }
    }

    async fn reply(stream: &mut TcpStream, status: &str, extra: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.expect("write");
    }

    /// The four requests one authenticated call takes: probe, `Type 1`,
    /// `Type 3`, then the real one. One connection only — a client that
    /// reconnects between messages cannot finish this.
    async fn ntlm_server(listener: TcpListener) {
        let (mut stream, _) = listener.accept().await.expect("accept");

        let (head, _) = read_request(&mut stream).await;
        assert!(
            !head.to_ascii_lowercase().contains("authorization"),
            "the first request must be unauthenticated"
        );
        reply(
            &mut stream,
            "401 Unauthorized",
            "WWW-Authenticate: Negotiate\r\nWWW-Authenticate: NTLM\r\n",
            "",
        )
        .await;

        let (head, _) = read_request(&mut stream).await;
        assert!(
            head.to_ascii_lowercase().contains("authorization: ntlm "),
            "the second request carries Type 1: {head}"
        );
        let challenge = base64::engine::general_purpose::STANDARD.encode(type2());
        reply(
            &mut stream,
            "401 Unauthorized",
            &format!("WWW-Authenticate: NTLM {challenge}\r\n"),
            "",
        )
        .await;

        let (head, body) = read_request(&mut stream).await;
        assert!(
            head.to_ascii_lowercase().contains("authorization: ntlm "),
            "the third request carries Type 3"
        );
        assert!(
            body.starts_with(b"<?xml"),
            "the third request still carries a SOAP body"
        );
        reply(&mut stream, "200 OK", "Content-Type: text/xml\r\n", "").await;

        let (head, _) = read_request(&mut stream).await;
        // `hyper` writes header names in lower case.
        assert!(
            head.to_ascii_lowercase().contains(
                "soapaction: \"http://schemas.microsoft.com/exchange/services/2006/messages/findfolder\""
            ),
            "the real request names its action: {head}"
        );
        assert!(
            !head.to_ascii_lowercase().contains("authorization"),
            "an authenticated connection does not repeat the token"
        );
        reply(
            &mut stream,
            "200 OK",
            "Content-Type: text/xml\r\n",
            "<ok>done</ok>",
        )
        .await;
    }

    #[tokio::test]
    async fn four_requests_on_one_connection_end_in_success() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(ntlm_server(listener));

        let stream = Box::new(TcpStream::connect(address).await.unwrap()) as BoxedStream;
        let password = SecretValue::new("synthetic".to_owned());
        let credentials = Credentials::parse("Domain\\User", &password);
        let mut http = EwsHttp::with_stream(
            stream,
            "mail.example.com",
            "/EWS/Exchange.asmx",
            &credentials,
        )
        .await
        .expect("authentication succeeds");
        let body = http
            .post("FindFolder", "<soap:Envelope/>")
            .await
            .expect("post succeeds");
        assert_eq!(body, "<ok>done</ok>");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_wrong_password_ends_in_rejected() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let _ = read_request(&mut stream).await;
            reply(
                &mut stream,
                "401 Unauthorized",
                "WWW-Authenticate: NTLM\r\n",
                "",
            )
            .await;
            let _ = read_request(&mut stream).await;
            let challenge = base64::engine::general_purpose::STANDARD.encode(type2());
            reply(
                &mut stream,
                "401 Unauthorized",
                &format!("WWW-Authenticate: NTLM {challenge}\r\n"),
                "",
            )
            .await;
            let _ = read_request(&mut stream).await;
            reply(&mut stream, "401 Unauthorized", "", "").await;
        });

        let stream = Box::new(TcpStream::connect(address).await.unwrap()) as BoxedStream;
        let password = SecretValue::new("wrong".to_owned());
        let credentials = Credentials::parse("Domain\\User", &password);
        let error = EwsHttp::with_stream(
            stream,
            "mail.example.com",
            "/EWS/Exchange.asmx",
            &credentials,
        )
        .await
        .err()
        .expect("authentication fails");
        assert_eq!(error, HttpError::Rejected);
    }

    #[tokio::test]
    async fn a_server_without_ntlm_is_reported_as_such() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let _ = read_request(&mut stream).await;
            reply(
                &mut stream,
                "401 Unauthorized",
                "WWW-Authenticate: Basic realm=\"x\"\r\n",
                "",
            )
            .await;
        });

        let stream = Box::new(TcpStream::connect(address).await.unwrap()) as BoxedStream;
        let password = SecretValue::new("synthetic".to_owned());
        let credentials = Credentials::parse("Domain\\User", &password);
        let error = EwsHttp::with_stream(
            stream,
            "mail.example.com",
            "/EWS/Exchange.asmx",
            &credentials,
        )
        .await
        .err()
        .expect("authentication fails");
        assert_eq!(error, HttpError::NoNtlm);
    }

    #[test]
    fn the_challenge_is_read_from_the_ntlm_scheme_only() {
        let challenge = base64::engine::general_purpose::STANDARD.encode([b'x'; 4]);
        let mut headers = hyper::HeaderMap::new();
        headers.append(
            WWW_AUTHENTICATE,
            format!("Negotiate, NTLM {challenge}").parse().unwrap(),
        );
        assert_eq!(challenge_token(&headers).unwrap(), Some(vec![b'x'; 4]));

        let mut headers = hyper::HeaderMap::new();
        headers.append(WWW_AUTHENTICATE, "NTLM".parse().unwrap());
        assert_eq!(challenge_token(&headers).unwrap(), None);

        let mut headers = hyper::HeaderMap::new();
        headers.append(WWW_AUTHENTICATE, "Negotiate".parse().unwrap());
        assert_eq!(challenge_token(&headers).unwrap_err(), HttpError::NoNtlm);
    }
}
