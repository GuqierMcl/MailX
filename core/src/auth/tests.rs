use super::*;
struct Identity;
impl IdentityVerifier for Identity {
    fn verify<'a>(
        &'a self,
        _: &'a IdentityEvidence<'a>,
        _: &'a NetRoute,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<VerifiedIdentity, OAuthError>> + Send + 'a>,
    > {
        Box::pin(async {
            Ok(VerifiedIdentity {
                issuer: "https://issuer.fixture.invalid".into(),
                subject: "subject".into(),
                mailbox: "test@example.com".into(),
            })
        })
    }
}
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn registration(port: u16) -> ProviderRegistration {
    ProviderRegistration {
        id: "test".into(),
        client_id: "public-client".into(),
        registration_id: "test-registration".into(),
        mail_policy: MailEndpointPolicy {
            imap: MailProtocolPolicy {
                host: "imap.example.com".into(),
                port: 993,
                tls: crate::TlsMode::Implicit,
                login: MailLoginPolicy::Mailbox,
            },
            smtp: MailProtocolPolicy {
                host: "smtp.example.com".into(),
                port: 465,
                tls: crate::TlsMode::Implicit,
                login: MailLoginPolicy::Mailbox,
            },
        },
        authorization_endpoint: Url::parse(&format!("http://127.0.0.1:{port}/authorize")).unwrap(),
        token_endpoint: Url::parse(&format!("http://127.0.0.1:{port}/token")).unwrap(),
        scopes: vec!["mail".into()],
        resource: None,
        allowed_targets: vec![],
        redirect_path: "/callback".into(),
        extra_authorize_params: vec![],
        login_hint: false,
    }
}
#[test]
fn mail_connections_come_from_the_registration_not_the_caller() {
    let mut registration = registration(1);
    registration.mail_policy.smtp.login = MailLoginPolicy::Fixed("relay".into());
    let (imap, smtp) = registration.mail_connections("  Lin@Example.com ");
    assert_eq!(imap.host, "imap.example.com");
    assert_eq!((imap.port, imap.tls), (993, crate::TlsMode::Implicit));
    assert_eq!(imap.username, "Lin@Example.com");
    assert_eq!(smtp.host, "smtp.example.com");
    assert_eq!((smtp.port, smtp.tls), (465, crate::TlsMode::Implicit));
    assert_eq!(smtp.username, "relay");
    registration
        .validate_mail_target("lin@example.com", &imap, &smtp)
        .unwrap();
}
async fn server(response: &'static str) -> (u16, tokio::task::JoinHandle<String>) {
    server_status(response, "200 OK").await
}
async fn server_status(
    response: &'static str,
    status: &'static str,
) -> (u16, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let n = stream.read(&mut chunk).await.unwrap();
            assert_ne!(n, 0);
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let len: usize = header
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + len {
                    break;
                }
            }
        }
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream.write_all(reply.as_bytes()).await.unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (port, task)
}
#[tokio::test]
async fn code_exchange_has_pkce_and_one_shot_state() {
    let (port, task) = server(r#"{"access_token":"access-marker","refresh_token":"refresh-marker","token_type":"Bearer","expires_in":3600}"#).await;
    let redirect = Url::parse("http://127.0.0.1:41234/callback").unwrap();
    let (session, authorization) =
        OAuthSession::begin(registration(port), redirect.as_str(), None).unwrap();
    let params: std::collections::HashMap<_, _> =
        authorization.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["redirect_uri"], redirect.as_str());
    let mut callback = redirect;
    callback
        .query_pairs_mut()
        .extend_pairs([("state", params["state"].as_str()), ("code", "code-marker")]);
    let authorization = session
        .complete(
            &callback,
            &NetRoute::default(),
            &Identity,
            "test@example.com",
        )
        .await
        .unwrap();
    let tokens = authorization.tokens;
    assert_eq!(tokens.access_token.expose_secret(), "access-marker");
    assert!(!format!("{tokens:?}").contains("access-marker"));
    let request = task.await.unwrap();
    assert!(request.contains("code_verifier="));
    assert!(request.contains("grant_type=authorization_code"));
    // Byte-identical to the authorization request's value.
    assert!(request.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A41234%2Fcallback&"));
}
#[tokio::test]
async fn mismatched_and_duplicate_state_reject_before_network() {
    let redirect = Url::parse("http://127.0.0.1:41234/callback").unwrap();
    let (session, _) = OAuthSession::begin(registration(9), redirect.as_str(), None).unwrap();
    let callback = Url::parse("http://127.0.0.1:41234/callback?state=wrong&code=secret").unwrap();
    assert_eq!(
        session
            .complete(
                &callback,
                &NetRoute::default(),
                &Identity,
                "test@example.com"
            )
            .await
            .err()
            .unwrap(),
        OAuthError::InvalidCallback
    );
    let (session, authorization) =
        OAuthSession::begin(registration(9), redirect.as_str(), None).unwrap();
    let state = authorization
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let mut callback = redirect;
    callback.query_pairs_mut().extend_pairs([
        ("state", state.as_str()),
        ("state", state.as_str()),
        ("code", "secret"),
    ]);
    assert_eq!(
        session
            .complete(
                &callback,
                &NetRoute::default(),
                &Identity,
                "test@example.com"
            )
            .await
            .err()
            .unwrap(),
        OAuthError::InvalidCallback
    );
}
#[tokio::test]
async fn concurrent_refresh_is_singleflight_and_persists_rotation() {
    use crate::secrets::test_support::MemorySecrets;
    let (port, task) = server(r#"{"access_token":"new-access","refresh_token":"rotated-refresh","token_type":"Bearer","expires_in":3600}"#).await;
    let store = Arc::new(MemorySecrets::default());
    store
        .set(
            "test-grant",
            &TokenSet {
                access_token: SecretValue::new("expired".into()),
                refresh_token: Some(SecretValue::new("old-refresh".into())),
                expires_at: 0,
                target: None,
                scopes: vec!["mail".into()],
            }
            .encode_secret()
            .unwrap(),
        )
        .unwrap();
    let manager = Arc::new(
        TokenManager::new(registration(port), store.clone(), "test-grant".into()).unwrap(),
    );
    let route = NetRoute::default();
    let (a, b) = tokio::join!(manager.access_token(&route), manager.access_token(&route));
    assert_eq!(a.unwrap().expose_secret(), "new-access");
    assert_eq!(b.unwrap().expose_secret(), "new-access");
    assert_eq!(
        TokenSet::decode_secret(&store.get("test-grant").unwrap().unwrap())
            .unwrap()
            .refresh_token
            .unwrap()
            .expose_secret(),
        "rotated-refresh"
    );
    assert!(task.await.unwrap().contains("refresh_token=old-refresh"));
}
#[test]
fn untrusted_registration_is_rejected() {
    let mut reg = registration(1);
    reg.token_endpoint = Url::parse("http://example.com/token").unwrap();
    assert_eq!(reg.validate(), Err(OAuthError::InvalidRegistration));
}

#[tokio::test]
async fn concurrent_failed_refreshes_share_one_attempt() {
    use crate::secrets::test_support::MemorySecrets;
    let (port, task) =
        server(r#"{"access_token":"bad","token_type":"mac","expires_in":3600}"#).await;
    let store = Arc::new(MemorySecrets::default());
    let mut manager = TokenManager::new(registration(port), store, "failure-grant".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("refresh".into())),
            expires_at: 0,
            target: None,
            scopes: vec!["mail".into()],
        })
        .unwrap();
    let manager = Arc::new(manager);
    let route = NetRoute::default();
    let (a, b) = tokio::join!(manager.access_token(&route), manager.access_token(&route));
    assert_eq!(a.unwrap_err(), OAuthError::InvalidResponse);
    assert_eq!(b.unwrap_err(), OAuthError::InvalidResponse);
    task.await.unwrap();
}

#[tokio::test]
async fn cancelled_waiter_does_not_lose_rotated_refresh_token() {
    use crate::secrets::test_support::MemorySecrets;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (received_tx, received_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        received_tx.send(()).unwrap();
        release_rx.await.unwrap();
        let body = r#"{"access_token":"new-access","refresh_token":"rotated","token_type":"Bearer","expires_in":3600}"#;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let store = Arc::new(MemorySecrets::default());
    let mut manager =
        TokenManager::new(registration(port), store.clone(), "cancel-grant".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("refresh".into())),
            expires_at: 0,
            target: None,
            scopes: vec!["mail".into()],
        })
        .unwrap();
    let manager = Arc::new(manager);
    let waiter_manager = manager.clone();
    let waiter =
        tokio::spawn(async move { waiter_manager.access_token(&NetRoute::default()).await });
    received_rx.await.unwrap();
    waiter.abort();
    release_tx.send(()).unwrap();
    manager.drain().await;
    server.await.unwrap();
    let saved = TokenSet::decode_secret(&store.get("cancel-grant").unwrap().unwrap()).unwrap();
    assert_eq!(saved.refresh_token.unwrap().expose_secret(), "rotated");
    manager.invalidate();
    assert!(manager.cached_token().is_none());
    assert_eq!(
        manager
            .access_token(&NetRoute::default())
            .await
            .unwrap_err(),
        OAuthError::ReauthenticationRequired
    );
}

#[tokio::test]
async fn loopback_callback_is_bound_and_consumed() {
    let callback = super::loopback::LoopbackCallback::bind("/callback")
        .await
        .unwrap();
    let redirect = Url::parse(callback.redirect_uri()).unwrap();
    let receiver = tokio::spawn(callback.receive());
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", redirect.port().unwrap()))
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "GET {}?code=one&state=two HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
                redirect.path(),
                redirect.port().unwrap()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let result = receiver.await.unwrap().unwrap();
    assert_eq!(result.path(), redirect.path());
    assert_eq!(result.query(), Some("code=one&state=two"));
}

#[tokio::test]
async fn verified_identity_must_match_the_target_mailbox() {
    let (port, server) = server(r#"{"access_token":"wrong-user-token","refresh_token":"refresh","token_type":"Bearer","expires_in":3600}"#).await;
    let redirect = Url::parse("http://127.0.0.1:41234/callback").unwrap();
    let (session, authorization) =
        OAuthSession::begin(registration(port), redirect.as_str(), None).unwrap();
    let state = authorization
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let mut callback = redirect;
    callback
        .query_pairs_mut()
        .extend_pairs([("state", state.as_str()), ("code", "code")]);
    assert_eq!(
        session
            .complete(
                &callback,
                &NetRoute::default(),
                &Identity,
                "other@example.com"
            )
            .await
            .err(),
        Some(OAuthError::InvalidResponse)
    );
    server.await.unwrap();
}

#[tokio::test]
async fn insufficient_scope_is_rejected() {
    let (port, server) = server(r#"{"access_token":"access","refresh_token":"refresh","token_type":"Bearer","expires_in":3600,"scope":"profile"}"#).await;
    let error = transport::exchange(
        &NetRoute::default(),
        &registration(port).token_endpoint,
        "grant_type=refresh_token".into(),
        None,
        &["mail".into()],
    )
    .await
    .unwrap_err();
    assert_eq!(error, OAuthError::InvalidResponse);
    server.await.unwrap();
}

#[tokio::test]
async fn registered_loopback_path_rejects_unrelated_requests_and_cancels() {
    let callback = loopback::LoopbackCallback::bind("/callback").await.unwrap();
    let redirect = Url::parse(callback.redirect_uri()).unwrap();
    assert_eq!(redirect.path(), "/callback");
    let port = redirect.port().unwrap();
    let receiver = tokio::spawn(callback.receive());
    let mut wrong = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    wrong
        .write_all(format!("GET /unrelated HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = [0; 16];
    assert_eq!(wrong.read(&mut response).await.unwrap(), 0);
    assert!(!receiver.is_finished());
    receiver.abort();
    let _ = receiver.await;
    assert!(
        tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn empty_callback_path_has_no_trailing_slash_and_accepts_the_root_request() {
    let callback = loopback::LoopbackCallback::bind("").await.unwrap();
    let uri = callback.redirect_uri().to_owned();
    let port = Url::parse(&uri).unwrap().port().unwrap();
    assert_eq!(uri, format!("http://127.0.0.1:{port}"));
    let receiver = tokio::spawn(callback.receive());
    let mut browser = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    browser
        .write_all(
            format!("GET /?code=one&state=two HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut page = String::new();
    browser.read_to_string(&mut page).await.unwrap();
    assert_eq!(
        receiver.await.unwrap().unwrap().query(),
        Some("code=one&state=two")
    );
    assert!(page.contains("text/html"));
    assert!(page.contains("Sign-in complete") && page.contains("登录完成"));
}

#[tokio::test]
async fn callback_page_is_script_free_and_differs_when_the_provider_reports_an_error() {
    let callback = loopback::LoopbackCallback::bind("/callback").await.unwrap();
    let port = Url::parse(callback.redirect_uri()).unwrap().port().unwrap();
    let receiver = tokio::spawn(callback.receive());
    let mut browser = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    browser
        .write_all(
            format!(
                "GET /callback?error=access_denied&state=s HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut page = String::new();
    browser.read_to_string(&mut page).await.unwrap();
    receiver.await.unwrap().unwrap();
    assert!(page.contains("Sign-in was not completed") && page.contains("未能完成登录"));
    assert!(!page.contains("access_denied"));
    assert!(!page.to_lowercase().contains("<script"));
    assert!(page.contains("Content-Security-Policy: default-src 'none'"));
    assert!(!page.contains("script-src"));
}

#[test]
fn redirect_uri_text_is_built_once_from_the_registration() {
    let mut reg = registration(1);
    assert_eq!(reg.redirect_uri(53124), "http://127.0.0.1:53124/callback");
    reg.redirect_path = String::new();
    assert_eq!(reg.redirect_uri(53124), "http://127.0.0.1:53124");
    assert_eq!(reg.validate(), Ok(()));
    for bad in ["callback", "//x", "/a b", "/a?b", "/a#b", "/a\\b"] {
        reg.redirect_path = bad.into();
        assert_eq!(
            reg.validate(),
            Err(OAuthError::InvalidRegistration),
            "{bad}"
        );
    }
    // A redirect that differs from the registered value is refused.
    let reg = registration(1);
    assert_eq!(
        OAuthSession::begin(reg.clone(), "http://127.0.0.1:41234/other", None).err(),
        Some(OAuthError::InvalidRegistration)
    );
    assert_eq!(
        OAuthSession::begin(reg, "http://127.0.0.1:41234/callback/", None).err(),
        Some(OAuthError::InvalidRegistration)
    );
}

#[test]
fn authorize_extras_are_static_and_the_login_hint_is_opt_in() {
    let mut reg = registration(1);
    reg.extra_authorize_params = vec![("prompt".into(), "select_account".into())];
    let uri = "http://127.0.0.1:41234/callback";
    let (_, url) = OAuthSession::begin(reg.clone(), uri, Some("lin@example.com")).unwrap();
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(params["prompt"], "select_account");
    assert!(!params.contains_key("login_hint"));
    reg.login_hint = true;
    let (_, url) = OAuthSession::begin(reg.clone(), uri, Some("lin@example.com")).unwrap();
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(params["login_hint"], "lin@example.com");
    assert_eq!(params["redirect_uri"], uri);
    assert_eq!(
        OAuthSession::begin(reg.clone(), uri, Some("a\nb@example.com")).err(),
        Some(OAuthError::InvalidRegistration)
    );
    // Extras cannot override parameters the flow owns.
    for key in [
        "state",
        "redirect_uri",
        "scope",
        "code_challenge",
        "login_hint",
    ] {
        let mut reg = registration(1);
        reg.extra_authorize_params = vec![(key.into(), "x".into())];
        assert_eq!(
            reg.validate(),
            Err(OAuthError::InvalidRegistration),
            "{key}"
        );
    }
}

#[tokio::test]
async fn speculative_empty_connection_does_not_block_callback() {
    let callback = loopback::LoopbackCallback::bind("/callback").await.unwrap();
    let redirect = Url::parse(callback.redirect_uri()).unwrap();
    let port = redirect.port().unwrap();
    let receiver = tokio::spawn(callback.receive());
    let _empty = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let mut valid = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    valid
        .write_all(
            format!(
                "GET {}?code=one&state=two HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n",
                redirect.path()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .query(),
        Some("code=one&state=two")
    );
}

#[derive(Default)]
struct FallibleSecrets {
    memory: crate::secrets::test_support::MemorySecrets,
    fail_set: std::sync::atomic::AtomicBool,
}
impl SecretStore for FallibleSecrets {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, crate::secrets::SecretStoreError> {
        self.memory.get(key)
    }
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), crate::secrets::SecretStoreError> {
        if self.fail_set.load(std::sync::atomic::Ordering::SeqCst) {
            Err(crate::secrets::SecretStoreError::Locked)
        } else {
            self.memory.set(key, value)
        }
    }
    fn delete(&self, key: &str) -> Result<(), crate::secrets::SecretStoreError> {
        self.memory.delete(key)
    }
}
#[tokio::test]
async fn keychain_failure_retains_unpublished_rotation_for_durable_retry() {
    let (port, server)=server(r#"{"access_token":"new-access","refresh_token":"rotated","token_type":"Bearer","expires_in":3600}"#).await;
    let store = Arc::new(FallibleSecrets::default());
    let mut manager =
        TokenManager::new(registration(port), store.clone(), "failed-keychain".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("old-refresh".into())),
            expires_at: 0,
            target: None,
            scopes: vec!["mail".into()],
        })
        .unwrap();
    let manager = Arc::new(manager);
    store
        .fail_set
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        manager
            .access_token(&NetRoute::default())
            .await
            .unwrap_err(),
        OAuthError::Storage
    );
    manager.drain().await;
    server.await.unwrap();
    assert!(!manager.needs_reauthorization());
    assert!(manager.cached_token().is_none());
    store
        .fail_set
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        manager
            .access_token(&NetRoute::default())
            .await
            .unwrap()
            .expose_secret(),
        "new-access"
    );
    assert_eq!(
        TokenSet::decode_secret(&store.get("failed-keychain").unwrap().unwrap())
            .unwrap()
            .refresh_token
            .unwrap()
            .expose_secret(),
        "rotated"
    );
}
#[tokio::test]
async fn proxy_failure_never_falls_back_to_direct_token_endpoint() {
    for kind in [crate::ProxyKind::HttpConnect, crate::ProxyKind::Socks5] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let dead = crate::net::test_support::dead_port().await;
        let route = crate::net::test_support::proxy_route(kind, dead);
        assert_eq!(
            transport::exchange(
                &route,
                &registration(port).token_endpoint,
                "secret-marker".into(),
                None,
                &["mail".into()]
            )
            .await
            .unwrap_err(),
            OAuthError::Proxy
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn definitive_invalid_grant_is_persisted_and_survives_restart() {
    let (port, server) = server_status(
        r#"{"error":"invalid_grant","error_description":"Bearer raw-secret-and-encoded-marker"}"#,
        "400 Bad Request",
    )
    .await;
    let store = Arc::new(crate::secrets::test_support::MemorySecrets::default());
    let mut manager =
        TokenManager::new(registration(port), store.clone(), "revoked-grant".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("old-refresh".into())),
            expires_at: 0,
            target: None,
            scopes: vec!["mail".into()],
        })
        .unwrap();
    let manager = Arc::new(manager);
    let error = manager
        .access_token(&NetRoute::default())
        .await
        .unwrap_err();
    assert_eq!(error, OAuthError::ReauthenticationRequired);
    assert!(!format!("{error:?}").contains("raw-secret"));
    manager.drain().await;
    server.await.unwrap();
    assert!(manager.needs_reauthorization());
    let saved = TokenSet::decode_secret(&store.get("revoked-grant").unwrap().unwrap()).unwrap();
    assert!(saved.refresh_token.is_none());
    let mut reopened =
        TokenManager::new(registration(port), store, "revoked-grant".into()).unwrap();
    reopened.restore(saved).unwrap();
    assert!(reopened.needs_reauthorization());
    assert!(reopened.cached_token().is_none());
}
#[tokio::test]
async fn transient_http_errors_are_distinct_and_never_contain_peer_text() {
    for (status, expected) in [
        ("429 Too Many Requests", OAuthError::RateLimited),
        ("503 Unavailable", OAuthError::ServerUnavailable),
    ] {
        let (port, server) = server_status(
            r#"{"error":"invalid_grant","error_description":"raw-secret-marker"}"#,
            status,
        )
        .await;
        let error = transport::exchange(
            &NetRoute::default(),
            &registration(port).token_endpoint,
            "grant_type=refresh_token".into(),
            None,
            &["mail".into()],
        )
        .await
        .unwrap_err();
        assert_eq!(error, expected);
        assert!(!format!("{error:?}").contains("raw-secret"));
        server.await.unwrap();
    }
}

#[derive(Default)]
struct BlockingSecrets {
    memory: crate::secrets::test_support::MemorySecrets,
    started: tokio::sync::Notify,
    release: (std::sync::Mutex<bool>, std::sync::Condvar),
    fail_delete: std::sync::atomic::AtomicBool,
}
impl SecretStore for BlockingSecrets {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, crate::secrets::SecretStoreError> {
        self.memory.get(key)
    }
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), crate::secrets::SecretStoreError> {
        self.started.notify_one();
        let mut ready = self.release.0.lock().unwrap();
        while !*ready {
            ready = self.release.1.wait(ready).unwrap();
        }
        self.memory.set(key, value)
    }
    fn delete(&self, key: &str) -> Result<(), crate::secrets::SecretStoreError> {
        if self.fail_delete.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(crate::secrets::SecretStoreError::Unavailable);
        }
        self.memory.delete(key)
    }
}
#[tokio::test]
async fn invalidated_grant_cannot_resurrect_during_blocked_keychain_write() {
    let(port,server)=server(r#"{"access_token":"new-access","refresh_token":"rotated","token_type":"Bearer","expires_in":3600}"#).await;
    let store = Arc::new(BlockingSecrets::default());
    let mut manager = TokenManager::new(
        registration(port),
        store.clone(),
        "old-generation-grant".into(),
    )
    .unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("refresh".into())),
            expires_at: 0,
            target: None,
            scopes: vec!["mail".into()],
        })
        .unwrap();
    let manager = Arc::new(manager);
    let worker = manager.clone();
    let waiter = tokio::spawn(async move { worker.access_token(&NetRoute::default()).await });
    store.started.notified().await;
    manager.invalidate();
    *store.release.0.lock().unwrap() = true;
    store.release.1.notify_all();
    assert_eq!(
        waiter.await.unwrap().unwrap_err(),
        OAuthError::ReauthenticationRequired
    );
    manager.drain().await;
    server.await.unwrap();
    assert!(store.get("old-generation-grant").unwrap().is_none());
    assert!(manager.cached_token().is_none());
}

#[tokio::test]
async fn one_grant_isolates_resource_scope_caches_and_shares_rotations() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let mut records = vec![];
        for n in 1..=2 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = tokio::io::BufReader::new(stream);
            use tokio::io::AsyncBufReadExt;
            let mut line = String::new();
            let mut len = 0;
            loop {
                line.clear();
                stream.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_lowercase().strip_prefix("content-length: ") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut bytes = vec![0; len];
            stream.read_exact(&mut bytes).await.unwrap();
            let form: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(&bytes).into_owned().collect();
            assert_eq!(
                form["refresh_token"],
                if n == 1 {
                    "original-refresh"
                } else {
                    "rotation-1"
                }
            );
            let resource = form["resource"].clone();
            let scope = form["scope"].clone();
            let body=serde_json::json!({"access_token":format!("access-{resource}"),"refresh_token":format!("rotation-{n}"),"token_type":"Bearer","expires_in":3600,"scope":scope}).to_string();
            stream
                .get_mut()
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            records.push(form);
        }
        records
    });
    let mut registration = registration(port);
    registration.resource = Some("urn:imap".into());
    registration.scopes = vec!["read".into(), "mail".into()];
    let imap = TokenTarget::new(Some("urn:imap".into()), vec!["mail".into(), "read".into()]);
    let smtp = TokenTarget::new(Some("urn:smtp".into()), vec!["smtp".into()]);
    registration.allowed_targets = vec![smtp.clone()];
    let store = Arc::new(crate::secrets::test_support::MemorySecrets::default());
    let mut manager = TokenManager::new(
        registration.clone(),
        store.clone(),
        "multi-target-grant".into(),
    )
    .unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("original-refresh".into())),
            expires_at: 0,
            target: None,
            scopes: vec!["mail".into(), "read".into()],
        })
        .unwrap();
    let manager = Arc::new(manager);
    let route = NetRoute::default();
    let (a, a2, b) = tokio::join!(
        manager.access_token_for(imap.clone(), &route),
        manager.access_token_for(imap.clone(), &route),
        manager.access_token_for(smtp.clone(), &route)
    );
    assert_eq!(a.unwrap().expose_secret(), "access-urn:imap");
    assert_eq!(a2.unwrap().expose_secret(), "access-urn:imap");
    assert_eq!(b.unwrap().expose_secret(), "access-urn:smtp");
    manager.drain().await;
    assert_eq!(
        manager
            .access_token_for(imap, &route)
            .await
            .unwrap()
            .expose_secret(),
        "access-urn:imap"
    );
    assert_eq!(manager.cached.read().unwrap().len(), 2);
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["scope"], "mail read");
    assert_eq!(requests[1]["resource"], "urn:smtp");
    let saved =
        TokenSet::decode_secret(&store.get("multi-target-grant").unwrap().unwrap()).unwrap();
    assert_eq!(saved.target, Some(smtp));
    assert_eq!(
        saved.refresh_token.as_ref().unwrap().expose_secret(),
        "rotation-2"
    );
    let mut reopened = TokenManager::new(registration, store, "multi-target-grant".into()).unwrap();
    reopened.restore(saved).unwrap();
    assert!(reopened.cached_token().is_none());
    assert_eq!(
        manager
            .access_token_for(
                TokenTarget::new(Some("urn:untrusted".into()), vec!["mail".into()]),
                &route
            )
            .await
            .unwrap_err(),
        OAuthError::InvalidRegistration
    );
}

#[tokio::test]
async fn failed_compensation_remains_a_storage_failure_until_account_cleanup() {
    let(port,server)=server(r#"{"access_token":"new-access","refresh_token":"rotated","token_type":"Bearer","expires_in":3600}"#).await;
    let store = Arc::new(BlockingSecrets::default());
    let mut manager =
        TokenManager::new(registration(port), store.clone(), "retired-grant".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("refresh".into())),
            expires_at: 0,
            target: None,
            scopes: vec!["mail".into()],
        })
        .unwrap();
    let manager = Arc::new(manager);
    let worker = manager.clone();
    let waiter = tokio::spawn(async move { worker.access_token(&NetRoute::default()).await });
    store.started.notified().await;
    assert!(!manager.is_drained());
    manager.invalidate();
    store
        .fail_delete
        .store(true, std::sync::atomic::Ordering::SeqCst);
    *store.release.0.lock().unwrap() = true;
    store.release.1.notify_all();
    assert_eq!(waiter.await.unwrap().unwrap_err(), OAuthError::Storage);
    manager.drain().await;
    assert!(manager.is_drained());
    server.await.unwrap();
    // Failure is not silently acknowledged. The account's durable cleanup
    // intent must retain this unique retired key for retry after drain.
    assert!(store.get("retired-grant").unwrap().is_some());
    assert!(manager.cached_token().is_none());
    store
        .fail_delete
        .store(false, std::sync::atomic::Ordering::SeqCst);
    store.delete("retired-grant").unwrap();
}
#[tokio::test]
async fn closed_admission_prevents_old_plan_refresh_without_revoking_stored_grant() {
    let store = Arc::new(crate::secrets::test_support::MemorySecrets::default());
    let tokens = TokenSet {
        access_token: SecretValue::new("expired".into()),
        refresh_token: Some(SecretValue::new("refresh".into())),
        expires_at: 0,
        target: None,
        scopes: vec!["mail".into()],
    };
    store
        .set("shutdown-grant", &tokens.encode_secret().unwrap())
        .unwrap();
    let mut manager =
        TokenManager::new(registration(9), store.clone(), "shutdown-grant".into()).unwrap();
    manager.restore(tokens).unwrap();
    let manager = Arc::new(manager);
    let old_plan = super::credential::ProtocolCredential::ManagedOAuth2(manager.clone());
    manager.close_admission().unwrap();
    manager.drain().await;
    assert!(old_plan.resolve(&NetRoute::default()).await.is_err());
    assert!(manager.is_drained());
    assert!(manager.is_valid());
    assert!(!manager.needs_reauthorization());
    assert!(store.get("shutdown-grant").unwrap().is_some());
}

#[test]
fn trusted_mail_policy_binds_servers_tls_and_both_sasl_identities() {
    let registration = registration(9);
    let imap = crate::ConnectionConfig {
        host: "imap.example.com".into(),
        port: 993,
        tls: crate::TlsMode::Implicit,
        username: "lin@example.com".into(),
    };
    let smtp = crate::ConnectionConfig {
        host: "smtp.example.com".into(),
        port: 465,
        tls: crate::TlsMode::Implicit,
        username: "lin@example.com".into(),
    };
    assert!(
        registration
            .validate_mail_target("  Lin@EXAMPLE.COM  ", &imap, &smtp)
            .is_ok()
    );
    let mut equivalent_case = imap.clone();
    equivalent_case.username = "Lin@EXAMPLE.COM".into();
    assert!(
        registration
            .validate_mail_target("lin@example.com", &equivalent_case, &smtp)
            .is_ok()
    );
    for attack in 0..5 {
        let mut imap = imap.clone();
        let mut smtp = smtp.clone();
        match attack {
            0 => smtp.host = "attacker.example".into(),
            1 => imap.host = "attacker.example".into(),
            2 => smtp.port = 587,
            3 => smtp.tls = crate::TlsMode::StartTls,
            _ => smtp.username = "other@example.com".into(),
        }
        assert_eq!(
            registration.validate_mail_target("lin@example.com", &imap, &smtp),
            Err(OAuthError::InvalidRegistration)
        );
    }
    let mut wrong_imap = imap.clone();
    wrong_imap.username = "other@example.com".into();
    assert_eq!(
        registration.validate_mail_target("lin@example.com", &wrong_imap, &smtp),
        Err(OAuthError::InvalidRegistration)
    );
    let mut fixed = registration.clone();
    fixed.mail_policy.imap.login = MailLoginPolicy::Fixed("tenant-specific-login".into());
    assert!(
        fixed
            .validate_mail_target("lin@example.com", &imap, &smtp)
            .is_err()
    );
    wrong_imap.username = "tenant-specific-login".into();
    assert!(
        fixed
            .validate_mail_target("lin@example.com", &wrong_imap, &smtp)
            .is_ok()
    );
}

#[tokio::test]
async fn shutdown_waits_for_keychain_recovery_before_discarding_a_received_rotation() {
    let (port,server)=server(r#"{"access_token":"new-access","refresh_token":"rotated-on-shutdown","token_type":"Bearer","expires_in":3600}"#).await;
    let store = Arc::new(FallibleSecrets::default());
    let mut manager =
        TokenManager::new(registration(port), store.clone(), "shutdown-pending".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("previous".into())),
            expires_at: 0,
            scopes: vec!["mail".into()],
            target: None,
        })
        .unwrap();
    let manager = Arc::new(manager);
    store
        .fail_set
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        manager
            .access_token(&NetRoute::default())
            .await
            .unwrap_err(),
        OAuthError::Storage
    );
    server.await.unwrap();
    manager.close_admission().unwrap();
    let owned = manager.clone();
    let settling = tokio::spawn(async move { owned.settle_persistence().await });
    tokio::task::yield_now().await;
    assert!(!settling.is_finished());
    store
        .fail_set
        .store(false, std::sync::atomic::Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(3), settling)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let saved = TokenSet::decode_secret(&store.get("shutdown-pending").unwrap().unwrap()).unwrap();
    assert_eq!(
        saved.refresh_token.unwrap().expose_secret(),
        "rotated-on-shutdown"
    );
    assert!(manager.is_drained());
}

#[tokio::test]
async fn persistent_keychain_failure_during_shutdown_has_bounded_retries() {
    let store = Arc::new(FallibleSecrets::default());
    let mut manager =
        TokenManager::new(registration(9), store.clone(), "persistently-locked".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("unpublished-access".into()),
            refresh_token: Some(SecretValue::new("unpublished-rotation".into())),
            expires_at: 0,
            scopes: vec!["mail".into()],
            target: None,
        })
        .unwrap();
    let manager = Arc::new(manager);
    manager
        .pending_persistence
        .store(true, std::sync::atomic::Ordering::SeqCst);
    store
        .fail_set
        .store(true, std::sync::atomic::Ordering::SeqCst);
    manager.close_admission().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(4), manager.settle_persistence())
        .await
        .unwrap();
    assert_eq!(result, Err(OAuthError::Storage));
    assert!(manager.is_drained());
    assert!(
        manager
            .pending_persistence
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[tokio::test]
async fn cancelling_a_shutdown_waiter_does_not_mark_an_in_progress_write_drained() {
    let store = Arc::new(BlockingSecrets::default());
    let mut manager =
        TokenManager::new(registration(9), store.clone(), "cancelled-shutdown".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("unpublished".into()),
            refresh_token: Some(SecretValue::new("rotation".into())),
            expires_at: 0,
            scopes: vec!["mail".into()],
            target: None,
        })
        .unwrap();
    manager
        .pending_persistence
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let manager = Arc::new(manager);
    manager.close_admission().unwrap();
    let waiting = manager.clone();
    let waiter = tokio::spawn(async move { waiting.settle_persistence().await });
    store.started.notified().await;
    waiter.abort();
    let _ = waiter.await;
    assert!(!manager.is_drained());
    *store.release.0.lock().unwrap() = true;
    store.release.1.notify_all();
    tokio::time::timeout(Duration::from_secs(3), manager.drain())
        .await
        .unwrap();
    assert!(store.get("cancelled-shutdown").unwrap().is_some());
}

#[tokio::test]
async fn an_unpolled_shutdown_owner_is_registered_before_its_waiter_can_be_cancelled() {
    use std::future::Future;
    let store = Arc::new(BlockingSecrets::default());
    let mut manager =
        TokenManager::new(registration(9), store.clone(), "queued-shutdown".into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("unpublished".into()),
            refresh_token: Some(SecretValue::new("rotation".into())),
            expires_at: 0,
            scopes: vec!["mail".into()],
            target: None,
        })
        .unwrap();
    manager
        .pending_persistence
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let manager = Arc::new(manager);
    let mut waiting = Box::pin(manager.settle_persistence());
    let waker = futures_util::task::noop_waker();
    assert!(
        waiting
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
            .is_pending()
    );
    assert!(!manager.is_drained());
    drop(waiting);
    assert!(!manager.is_drained());
    store.started.notified().await;
    *store.release.0.lock().unwrap() = true;
    store.release.1.notify_all();
    tokio::time::timeout(Duration::from_secs(3), manager.drain())
        .await
        .unwrap();
    assert!(store.get("queued-shutdown").unwrap().is_some());
}

#[test]
fn granted_scope_echo_may_omit_meta_scopes_but_not_resource_scopes() {
    let required = vec!["mail".to_owned(), "offline_access".to_owned()];
    assert_eq!(
        transport::granted_scopes(None, &required).unwrap(),
        required
    );
    let granted = transport::granted_scopes(Some(vec!["mail".into()]), &required).unwrap();
    assert!(granted.contains(&"mail".to_owned()));
    assert!(granted.contains(&"offline_access".to_owned()));
    assert_eq!(
        transport::granted_scopes(Some(vec!["offline_access".into()]), &required),
        Err(OAuthError::InvalidResponse)
    );
}

#[tokio::test]
async fn missing_expires_in_uses_a_short_default_lifetime() {
    let (port, server) =
        server(r#"{"access_token":"access","refresh_token":"refresh","token_type":"Bearer"}"#)
            .await;
    let tokens = transport::exchange(
        &NetRoute::default(),
        &registration(port).token_endpoint,
        "grant_type=refresh_token".into(),
        None,
        &["mail".into()],
    )
    .await
    .unwrap()
    .tokens;
    let lifetime = tokens.expires_at - now();
    assert!((1700..=1800).contains(&lifetime), "{lifetime}");
    server.await.unwrap();
}

#[tokio::test]
async fn rejected_access_token_is_forgotten_and_the_next_resolution_refreshes() {
    use crate::secrets::test_support::MemorySecrets;
    let (port, server) =
        server(r#"{"access_token":"replacement","token_type":"Bearer","expires_in":3600}"#).await;
    let mut manager = TokenManager::new(
        registration(port),
        Arc::new(MemorySecrets::default()),
        "rejected-grant".into(),
    )
    .unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("revoked".into()),
            refresh_token: Some(SecretValue::new("refresh".into())),
            expires_at: now() + 3600,
            target: Some(TokenTarget::new(None, vec!["mail".into()])),
            scopes: vec!["mail".into()],
        })
        .unwrap();
    let manager = Arc::new(manager);
    // An unrelated value leaves the cache alone.
    manager.reject_access_token(&SecretValue::new("other".into()));
    assert_eq!(manager.cached_token().unwrap().expose_secret(), "revoked");
    let credential = credential::ProtocolCredential::ManagedOAuth2(manager.clone())
        .resolve(&NetRoute::default())
        .await
        .unwrap();
    assert!(credential.reject());
    assert!(manager.cached_token().is_none());
    let token = manager.access_token(&NetRoute::default()).await.unwrap();
    assert_eq!(token.expose_secret(), "replacement");
    assert!(server.await.unwrap().contains("refresh_token=refresh"));
    // Password credentials have nothing to refresh.
    assert!(!credential::ProtocolCredential::Password(SecretValue::new("p".into())).reject());
}

/// Reads identity from the `id_token` only, as providers whose access tokens
/// the client must not parse require.
struct IdTokenIdentity;
impl IdentityVerifier for IdTokenIdentity {
    fn verify<'a>(
        &'a self,
        evidence: &'a IdentityEvidence<'a>,
        _: &'a NetRoute,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<VerifiedIdentity, OAuthError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let claims = evidence.id_token_claims()?;
            Ok(VerifiedIdentity {
                issuer: claims.issuer.clone(),
                subject: claims.subject.clone(),
                mailbox: claims
                    .string("email")
                    .ok_or(OAuthError::InvalidResponse)?
                    .into(),
            })
        })
    }
}
fn openid_registration(port: u16) -> ProviderRegistration {
    let mut reg = registration(port);
    reg.scopes = vec!["openid".into(), "email".into(), "mail".into()];
    reg
}
/// Runs a full code exchange whose token endpoint returns an `id_token` built
/// by `claims(nonce)`; returns the session result and the token request body.
async fn id_token_login(
    claims: impl Fn(&str) -> serde_json::Value,
    expected_mailbox: &str,
) -> (Result<VerifiedAuthorization, OAuthError>, String) {
    // The nonce is only known after `begin`, so serve the response lazily.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let redirect = Url::parse("http://127.0.0.1:41234/callback").unwrap();
    let (session, authorization) =
        OAuthSession::begin(openid_registration(port), redirect.as_str(), None).unwrap();
    let params: std::collections::HashMap<_, _> =
        authorization.query_pairs().into_owned().collect();
    let nonce = params["nonce"].clone();
    let id_token = id_token::fixture_token(claims(&nonce));
    let body = serde_json::json!({
        "access_token": "opaque-access-the-client-must-not-parse",
        "refresh_token": "refresh-marker",
        "token_type": "Bearer", "expires_in": 3600,
        "scope": "openid email mail",
        "id_token": id_token.expose_secret(),
    })
    .to_string();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let n = stream.read(&mut chunk).await.unwrap();
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + len {
                    break;
                }
            }
        }
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(reply.as_bytes()).await.unwrap();
        String::from_utf8(bytes).unwrap()
    });
    let mut callback = redirect;
    callback
        .query_pairs_mut()
        .extend_pairs([("state", params["state"].as_str()), ("code", "code")]);
    let result = session
        .complete(
            &callback,
            &NetRoute::default(),
            &IdTokenIdentity,
            expected_mailbox,
        )
        .await;
    (result, server.await.unwrap())
}
fn good_claims(nonce: &str) -> serde_json::Value {
    serde_json::json!({
        "iss": "https://login.fixture.invalid/tenant-42/v2.0", "sub": "subject-42",
        "aud": "public-client", "exp": now() + 3600, "nonce": nonce,
        "email": "test@example.com"
    })
}
#[tokio::test]
async fn identity_comes_from_the_id_token_with_a_per_tenant_issuer() {
    let (result, request) = id_token_login(good_claims, "test@example.com").await;
    let authorization = result.unwrap();
    assert_eq!(
        authorization.binding.issuer,
        "https://login.fixture.invalid/tenant-42/v2.0"
    );
    assert_eq!(authorization.binding.subject, "subject-42");
    assert_eq!(authorization.binding.mailbox, "test@example.com");
    // The id_token is neither part of the token set nor of its stored form.
    let stored = authorization.tokens.encode_secret().unwrap();
    assert!(!stored.expose_secret().contains("tenant-42"));
    assert!(!format!("{:?}", authorization.tokens).contains("tenant-42"));
    assert!(request.contains("grant_type=authorization_code"));
}
#[tokio::test]
async fn id_token_nonce_audience_and_expiry_are_enforced() {
    type Tamper = fn(&str) -> serde_json::Value;
    let cases: [(&str, Tamper); 3] = [
        ("nonce", |_| {
            let mut c = good_claims("other-nonce");
            c["nonce"] = "other-nonce".into();
            c
        }),
        ("audience", |n| {
            let mut c = good_claims(n);
            c["aud"] = "another-client".into();
            c
        }),
        ("expiry", |n| {
            let mut c = good_claims(n);
            c["exp"] = 1.into();
            c
        }),
    ];
    for (name, claims) in cases {
        let (result, _) = id_token_login(claims, "test@example.com").await;
        assert_eq!(result.err(), Some(OAuthError::InvalidResponse), "{name}");
    }
}
#[tokio::test]
async fn openid_requests_carry_a_fresh_nonce_and_others_do_not() {
    let redirect = "http://127.0.0.1:41234/callback";
    let nonce_of = |reg: ProviderRegistration| {
        let (_, url) = OAuthSession::begin(reg, redirect, None).unwrap();
        url.query_pairs()
            .find(|(k, _)| k == "nonce")
            .map(|(_, v)| v.into_owned())
    };
    let first = nonce_of(openid_registration(1)).unwrap();
    assert!(first.len() >= 20);
    assert_ne!(Some(first), nonce_of(openid_registration(1)));
    assert_eq!(nonce_of(registration(1)), None);
}

/// Models Windows Credential Manager through keyring: at most 2560 bytes per
/// item, stored as UTF-16, so 1280 characters. An oversized write fails and
/// stores nothing.
#[derive(Default)]
struct WindowsLimitSecrets {
    memory: crate::secrets::test_support::MemorySecrets,
}
impl SecretStore for WindowsLimitSecrets {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, crate::secrets::SecretStoreError> {
        self.memory.get(key)
    }
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), crate::secrets::SecretStoreError> {
        if value.expose_secret().encode_utf16().count() * 2 > 2560 {
            return Err(crate::secrets::SecretStoreError::Failed);
        }
        self.memory.set(key, value)
    }
    fn delete(&self, key: &str) -> Result<(), crate::secrets::SecretStoreError> {
        self.memory.delete(key)
    }
}
fn expired_grant(store: &Arc<WindowsLimitSecrets>, key: &str, port: u16) -> Arc<TokenManager> {
    let mut manager = TokenManager::new(registration(port), store.clone(), key.into()).unwrap();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("expired".into()),
            refresh_token: Some(SecretValue::new("old-refresh".into())),
            expires_at: 0,
            target: None,
            scopes: vec!["mail".into()],
        })
        .unwrap();
    Arc::new(manager)
}
fn token_body(access: &str, refresh: &str) -> &'static str {
    Box::leak(
        serde_json::json!({"access_token":access,"refresh_token":refresh,"token_type":"Bearer","expires_in":3600})
            .to_string()
            .into_boxed_str(),
    )
}
#[tokio::test]
async fn long_access_tokens_are_not_persisted_so_the_grant_fits_a_1280_character_item() {
    let access = "a".repeat(2000);
    let refresh = "r".repeat(900);
    let (port, server) = server(token_body(&access, &refresh)).await;
    let store = Arc::new(WindowsLimitSecrets::default());
    let manager = expired_grant(&store, "windows-grant", port);
    let token = manager.access_token(&NetRoute::default()).await.unwrap();
    assert_eq!(token.expose_secret(), access);
    manager.drain().await;
    server.await.unwrap();
    let stored = store.get("windows-grant").unwrap().unwrap();
    assert!(stored.expose_secret().encode_utf16().count() <= 1280);
    assert!(!stored.expose_secret().contains(&access[..50]));
    let restored = TokenSet::decode_secret(&stored).unwrap();
    assert_eq!(restored.refresh_token.unwrap().expose_secret(), refresh);
    // After a restart the access token is gone and the first use refreshes.
    let mut reopened = TokenManager::new(registration(1), store, "windows-grant".into()).unwrap();
    reopened
        .restore(TokenSet::decode_secret(&stored).unwrap())
        .unwrap();
    assert!(reopened.cached_token().is_none());
}
#[tokio::test]
async fn a_refresh_token_larger_than_the_store_limit_fails_with_storage_and_is_never_truncated() {
    let refresh = "r".repeat(2000);
    let (port, server) = server(token_body("short-access", &refresh)).await;
    let store = Arc::new(WindowsLimitSecrets::default());
    store
        .set("windows-grant", &expired_secret("old-refresh"))
        .unwrap();
    let manager = expired_grant(&store, "windows-grant", port);
    assert_eq!(
        manager
            .access_token(&NetRoute::default())
            .await
            .unwrap_err(),
        OAuthError::Storage
    );
    manager.drain().await;
    server.await.unwrap();
    // The previous value is intact: nothing was cut to fit.
    let kept = TokenSet::decode_secret(&store.get("windows-grant").unwrap().unwrap()).unwrap();
    assert_eq!(kept.refresh_token.unwrap().expose_secret(), "old-refresh");
    // The rotation stays in memory for a later write; no access token escaped.
    assert!(manager.cached_token().is_none());
    assert!(!manager.needs_reauthorization());
}
fn expired_secret(refresh: &str) -> SecretValue {
    TokenSet {
        access_token: SecretValue::new("expired".into()),
        refresh_token: Some(SecretValue::new(refresh.into())),
        expires_at: 0,
        target: None,
        scopes: vec!["mail".into()],
    }
    .encode_secret()
    .unwrap()
}
#[test]
fn stored_grants_hold_no_access_token_and_old_records_are_rejected_safely() {
    let tokens = TokenSet {
        access_token: SecretValue::new("access-marker".into()),
        refresh_token: Some(SecretValue::new("refresh-marker".into())),
        expires_at: 123,
        target: Some(TokenTarget::new(None, vec!["mail".into()])),
        scopes: vec!["mail".into()],
    };
    let stored = tokens.encode_secret().unwrap();
    assert!(!stored.expose_secret().contains("access-marker"));
    assert!(!stored.expose_secret().contains("expires_at"));
    let restored = TokenSet::decode_secret(&stored).unwrap();
    assert_eq!(restored.expires_at, 0);
    assert!(restored.access_token.expose_secret().is_empty());
    assert_eq!(restored.target, tokens.target);
    // The earlier layout (unreleased) fails closed instead of being half read.
    let legacy = SecretValue::new(
        r#"{"access_token":"a","refresh_token":"r","expires_at":1,"scopes":["mail"]}"#.into(),
    );
    assert_eq!(
        TokenSet::decode_secret(&legacy).err(),
        Some(OAuthError::Storage)
    );
}
#[tokio::test]
async fn token_endpoint_reports_reauthentication_for_interaction_errors() {
    for error in [
        "invalid_grant",
        "interaction_required",
        "consent_required",
        "login_required",
    ] {
        let body: &'static str = Box::leak(format!(r#"{{"error":"{error}"}}"#).into_boxed_str());
        let (port, server) = server_status(body, "400 Bad Request").await;
        let outcome = transport::exchange(
            &NetRoute::default(),
            &registration(port).token_endpoint,
            "grant_type=refresh_token".into(),
            None,
            &["mail".into()],
        )
        .await;
        assert_eq!(
            outcome.err(),
            Some(OAuthError::ReauthenticationRequired),
            "{error}"
        );
        server.await.unwrap();
    }
    let (port, server) = server_status(r#"{"error":"invalid_client"}"#, "400 Bad Request").await;
    let outcome = transport::exchange(
        &NetRoute::default(),
        &registration(port).token_endpoint,
        "grant_type=refresh_token".into(),
        None,
        &["mail".into()],
    )
    .await;
    assert_eq!(outcome.err(), Some(OAuthError::InvalidResponse));
    server.await.unwrap();
}
