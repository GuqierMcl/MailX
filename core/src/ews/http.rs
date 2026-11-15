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
/// What the two unauthenticated handshake requests carry. Their content is
/// irrelevant to authentication (the server refuses them before parsing the
/// body), but it must still be a well-formed EWS request: some Exchange / WAF
/// setups inspect the body first. The `m:` / `t:` prefixes matter — `BaseShape`
/// and `DistinguishedFolderId` belong to the *types* namespace, and putting
/// them in the messages namespace is what makes Exchange answer with
/// `ErrorInvalidOperation` ("ID 的格式不正确").
const PROBE_BODY: &str = concat!(
    r#"<?xml version="1.0" encoding="utf-8"?>"#,
    r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" "#,
    r#"xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages" "#,
    r#"xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">"#,
    r#"<soap:Header><t:RequestServerVersion Version="Exchange2013"/></soap:Header>"#,
    r#"<soap:Body><m:GetFolder>"#,
    r#"<m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape>"#,
    r#"<m:FolderIds><t:DistinguishedFolderId Id="inbox"/></m:FolderIds>"#,
    r#"</m:GetFolder></soap:Body></soap:Envelope>"#,
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
    /// EWS answered a request with a status this client does not treat as a
    /// success or a credential refusal, carrying a short response snippet so the
    /// caller can tell a 502 from a redirect or an HTML login page.
    UnexpectedStatus(u16, String),
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
        // The `Type 3` message authenticates the connection, and the server then
        // processes its body as a real EWS request. A malformed probe body (the
        // discovery `GetFolder` uses an unprefixed `BaseShape`, so the wrong
        // namespace) makes a *successful* login look like a 500 with an
        // `ErrorInvalidOperation` fault. Send the same `FindFolder` the caller
        // needs instead: on 200 we are authenticated and the caller's follow-up
        // `FindFolder` is a harmless repeat; on a fault the diagnostic names the
        // real operation rather than the handshake.
        let body = super::soap::find_folders();
        let response = self
            .send_with(Some(&type3), Some("FindFolder"), body)
            .await?;
        let status = response.status();
        if status.is_success() {
            // The server executed the FindFolder carried by Type 3 and answered
            // with a real SOAP body. It must be drained before `hyper` will let
            // us send the caller's next request on this single connection.
            drain(response).await?;
            return Ok(());
        }
        let status_code = status.as_u16();
        let snippet = diagnostic(response, "NTLM 认证后 FindFolder").await;
        Err(match status_code {
            401 | 403 => HttpError::Rejected,
            _ => HttpError::UnexpectedStatus(status_code, snippet),
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
            let status = response.status().as_u16();
            let snippet = diagnostic(response, action).await;
            return Err(HttpError::UnexpectedStatus(status, snippet));
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

/// Builds a diagnostic string for an unexpected non-2xx / non-401 response: the
/// HTTP status, the `Content-Type`, and a preview of the body. A SOAP fault or
/// an HTML login / error page in the body tells far more than a bare status
/// code. `label` names the request that produced the response (the NTLM
/// handshake or the EWS operation) so a 500 can be placed precisely.
async fn diagnostic(response: Response<Incoming>, label: &str) -> String {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("(无 Content-Type)")
        .to_owned();
    // Read with a generous bound, then show only the first 1 KiB. Using the
    // preview size as the `Limited` bound makes any larger body (a typical IIS
    // HTML error page) surface as a read error instead of its contents.
    let body = Limited::new(response.into_body(), 256 * 1024);
    let preview = match timeout(REQUEST_TIMEOUT, body.collect()).await {
        Ok(Ok(collected)) => {
            let bytes = collected.to_bytes();
            if bytes.is_empty() {
                "(空响应体)".to_owned()
            } else {
                let end = bytes.len().min(1024);
                let mut text = String::from_utf8_lossy(&bytes[..end]).to_string();
                if bytes.len() > end {
                    text.push_str("…(已截断)");
                }
                text
            }
        }
        Ok(Err(_)) => "(响应体读取失败)".to_owned(),
        Err(_) => "(响应体读取超时)".to_owned(),
    };
    format!("HTTP {status} · {content_type} · [{label}]：{preview}")
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
        let soap = String::from_utf8_lossy(&body);
        assert!(
            soap.starts_with("<?xml") && soap.contains("<m:FindFolder"),
            "Type 3 carries a real FindFolder request: {soap}"
        );
        assert!(
            soap.contains(r#"<t:DistinguishedFolderId Id="msgfolderroot"/>"#)
                && soap.contains("<t:BaseShape>Default</t:BaseShape>"),
            "the folder id and base shape use the types namespace: {soap}"
        );
        assert!(
            !soap.contains("<BaseShape>"),
            "the malformed, unprefixed BaseShape is gone: {soap}"
        );
        // A non-empty body: the client has to drain it before the fourth
        // request can go out on this same connection.
        reply(
            &mut stream,
            "200 OK",
            "Content-Type: text/xml\r\n",
            "<soap:Envelope><soap:Body/></soap:Envelope>",
        )
        .await;

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

    /// Regression for a real Exchange 2019 deployment: NTLM succeeded, but the
    /// server executed the malformed `GetFolder` probe carried by Type 3 and
    /// answered 500 with a SOAP Fault. The diagnostic must surface that body
    /// instead of a bare status code.
    #[tokio::test]
    async fn a_soap_fault_after_type3_is_reported_with_its_body() {
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
            let (head, body) = read_request(&mut stream).await;
            assert!(
                head.to_ascii_lowercase().contains("authorization: ntlm "),
                "Type 3 was sent"
            );
            let soap = String::from_utf8_lossy(&body);
            assert!(
                soap.contains("<m:FindFolder"),
                "Type 3 no longer carries the malformed GetFolder probe: {soap}"
            );
            let fault = concat!(
                r#"<?xml version="1.0" encoding="utf-8"?>"#,
                r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">"#,
                r#"<s:Body><s:Fault>"#,
                r#"<faultcode>a:ErrorInvalidOperation</faultcode>"#,
                r#"<faultstring>ID 的格式不正确。</faultstring>"#,
                r#"</s:Fault></s:Body></s:Envelope>"#,
            );
            reply(
                &mut stream,
                "500 Internal Server Error",
                "Content-Type: text/xml; charset=utf-8\r\n",
                fault,
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
        .expect("a 500 is a failure");
        let HttpError::UnexpectedStatus(status, detail) = error else {
            panic!("expected UnexpectedStatus, got {error:?}");
        };
        assert_eq!(status, 500);
        assert!(detail.contains("text/xml"), "{detail}");
        assert!(detail.contains("ErrorInvalidOperation"), "{detail}");
        assert!(detail.contains("FindFolder"), "{detail}");
    }

    /// The conservative `FindItem` retry must be possible on the *same*
    /// connection: NTLM is connection-bound, and a 500 whose body was consumed
    /// for diagnostics must not poison the hyper sender. This is the exact
    /// shape of a real `ErrorInvalidRequest` from an unknown `FieldURI`.
    #[tokio::test]
    async fn a_rejected_find_item_can_be_retried_on_the_same_connection() {
        use crate::ews::soap::{find_items, find_items_basic};

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
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
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                "<soap:Envelope><soap:Body/></soap:Envelope>",
            )
            .await;

            // First FindItem: the full property set is refused at the schema
            // level, so the answer is a top-level SOAP fault over HTTP 500.
            let (_, full) = read_request(&mut stream).await;
            let full = String::from_utf8_lossy(&full).to_string();
            assert!(full.contains("<m:FindItem"), "a FindItem was posted");
            assert!(full.contains(r#"FieldURI="item:Flag""#));
            let fault = concat!(
                r#"<?xml version="1.0" encoding="utf-8"?>"#,
                r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">"#,
                r#"<s:Body><s:Fault><faultcode>a:ErrorInvalidRequest</faultcode>"#,
                r#"<faultstring>请求无效。</faultstring>"#,
                r#"</s:Fault></s:Body></s:Envelope>"#,
            );
            reply(
                &mut stream,
                "500 Internal Server Error",
                "Content-Type: text/xml; charset=utf-8\r\n",
                fault,
            )
            .await;

            // Retry: only the original-schema paths, on the same connection.
            let (_, basic) = read_request(&mut stream).await;
            let basic = String::from_utf8_lossy(&basic).to_string();
            assert!(basic.contains("<m:FindItem"), "the retry is a FindItem");
            assert!(basic.contains(r#"FieldURI="item:Subject""#));
            for late in [
                "item:Flag",
                "item:InReplyTo",
                "message:References",
                "message:ReplyTo",
            ] {
                assert!(
                    !basic.contains(&format!("FieldURI=\"{late}\"")),
                    "the retry still asks for {late}"
                );
            }
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                "<ok>retried</ok>",
            )
            .await;
        });

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

        let error = http
            .post("FindItem", &find_items("inbox", 5))
            .await
            .expect_err("the full property set is refused");
        assert!(matches!(error, HttpError::UnexpectedStatus(500, _)));

        let body = http
            .post("FindItem", &find_items_basic("inbox", 5))
            .await
            .expect("the retry succeeds on the same connection");
        assert_eq!(body, "<ok>retried</ok>");
        server.await.unwrap();
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
