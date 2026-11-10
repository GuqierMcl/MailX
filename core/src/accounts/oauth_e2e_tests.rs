//! Fully local OAuth fixtures. No provider registration or token endpoint is
//! enabled outside cfg(test).
use super::*;
use crate::{
    auth::{
        IdentityVerifier, MailEndpointPolicy, MailLoginPolicy, MailProtocolPolicy, OAuthError,
        ProviderRegistration, TokenManager, TokenSet, VerifiedIdentity,
        credential::ProtocolCredential,
    },
    net::NetRoute,
    secrets::{SecretStore, test_support::MemorySecrets},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
use url::Url;

struct FixtureIdentity(u16);
impl IdentityVerifier for FixtureIdentity {
    fn verify<'a>(
        &'a self,
        token: &'a SecretValue,
        route: &'a NetRoute,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<VerifiedIdentity, OAuthError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let mut stream = crate::net::connect(route, "127.0.0.1", self.0)
                .await
                .map_err(|_| OAuthError::Network)?;
            stream.write_all(format!("GET /identity HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",token.expose_secret()).as_bytes()).await.map_err(|_|OAuthError::Network)?;
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .await
                .map_err(|_| OAuthError::Network)?;
            let body = response
                .split_once("\r\n\r\n")
                .ok_or(OAuthError::InvalidResponse)?
                .1;
            let identity: serde_json::Value =
                serde_json::from_str(body).map_err(|_| OAuthError::InvalidResponse)?;
            Ok(VerifiedIdentity {
                subject: identity["sub"]
                    .as_str()
                    .ok_or(OAuthError::InvalidResponse)?
                    .into(),
                mailbox: identity["email"]
                    .as_str()
                    .ok_or(OAuthError::InvalidResponse)?
                    .into(),
            })
        })
    }
}
async fn oauth_server(rotating: bool) -> (u16, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let exchanges = Arc::new(AtomicUsize::new(0));
    let count = exchanges.clone();
    let task = tokio::spawn(async move {
        for _ in 0..3 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut head = String::new();
            let mut line = String::new();
            let mut len = 0;
            loop {
                line.clear();
                stream.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                    len = value.trim().parse().unwrap();
                }
                head.push_str(&line);
            }
            let mut body = vec![0; len];
            stream.read_exact(&mut body).await.unwrap();
            let response = if head.starts_with("GET /identity ") {
                assert!(head.contains("Authorization: Bearer initial-access"));
                r#"{"sub":"verified-subject","email":"lin@example.com"}"#.to_owned()
            } else {
                assert!(head.starts_with("POST /token "));
                let n = count.fetch_add(1, Ordering::SeqCst);
                let body = String::from_utf8(body).unwrap();
                assert!(body.contains("resource=urn%3Amail"));
                if n == 0 {
                    assert!(body.contains("code_verifier="));
                    assert!(body.contains("code=fixture-code"));
                    r#"{"access_token":"initial-access","refresh_token":"stable-refresh","token_type":"Bearer","expires_in":3600,"scope":"mail"}"#.to_owned()
                } else {
                    assert!(body.contains("refresh_token=stable-refresh"));
                    format!(
                        r#"{{"access_token":"refreshed-access","refresh_token":"{}","token_type":"Bearer","expires_in":3600,"scope":"mail"}}"#,
                        if rotating {
                            "rotated-refresh"
                        } else {
                            "stable-refresh"
                        }
                    )
                }
            };
            stream.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).as_bytes()).await.unwrap();
        }
    });
    (port, exchanges, task)
}
async fn protocol_servers(
    tls: Arc<rustls::ServerConfig>,
) -> (
    u16,
    u16,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let imap = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let smtp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ip = imap.local_addr().unwrap().port();
    let sp = smtp.local_addr().unwrap().port();
    let itls = tls.clone();
    let imap_task = tokio::spawn(async move {
        for token in ["initial-access", "refreshed-access"] {
            let (stream, _) = imap.accept().await.unwrap();
            let stream = tokio_rustls::TlsAcceptor::from(itls.clone())
                .accept(stream)
                .await
                .unwrap();
            let mut stream = BufReader::new(stream);
            stream
                .get_mut()
                .write_all(b"* OK fixture ready\r\n")
                .await
                .unwrap();
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap().to_owned();
            assert!(line.contains("AUTHENTICATE XOAUTH2"));
            stream.get_mut().write_all(b"+ \r\n").await.unwrap();
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            assert_eq!(
                STANDARD.decode(line.trim()).unwrap(),
                format!("user=lin\x01auth=Bearer {token}\x01\x01").as_bytes()
            );
            stream
                .get_mut()
                .write_all(format!("{tag} OK authenticated\r\n").as_bytes())
                .await
                .unwrap();
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap();
            assert!(line.contains("CAPABILITY"));
            stream
                .get_mut()
                .write_all(
                    format!("* CAPABILITY IMAP4rev1 AUTH=XOAUTH2\r\n{tag} OK capability\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let smtp_task = tokio::spawn(async move {
        for token in ["initial-access", "refreshed-access"] {
            let (stream, _) = smtp.accept().await.unwrap();
            let stream = tokio_rustls::TlsAcceptor::from(tls.clone())
                .accept(stream)
                .await
                .unwrap();
            let mut stream = BufReader::new(stream);
            stream
                .get_mut()
                .write_all(b"220 fixture SMTP\r\n")
                .await
                .unwrap();
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            assert!(line.starts_with("EHLO "));
            stream
                .get_mut()
                .write_all(b"250-fixture\r\n250 AUTH XOAUTH2\r\n")
                .await
                .unwrap();
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            assert!(line.starts_with("AUTH XOAUTH2 "));
            let encoded = line.trim().strip_prefix("AUTH XOAUTH2 ").unwrap();
            assert_eq!(
                STANDARD.decode(encoded).unwrap(),
                format!("user=lin\x01auth=Bearer {token}\x01\x01").as_bytes()
            );
            stream
                .get_mut()
                .write_all(b"235 authenticated\r\n")
                .await
                .unwrap();
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            assert_eq!(line, "QUIT\r\n");
            stream.get_mut().write_all(b"221 bye\r\n").await.unwrap();
        }
    });
    (ip, sp, imap_task, smtp_task)
}
async fn authenticate_plans(
    service: &AccountService,
    id: &str,
    tls: &crate::net::test_support::TestTls,
) {
    let plan = service.sync_plan(id).unwrap();
    let _session = crate::imap::test_network_session(&plan.route, &plan.login, tls.client.clone())
        .await
        .unwrap();
    let smtp = service.smtp_plan(id).unwrap();
    crate::smtp::verify_connection(
        &smtp.route,
        &smtp.login,
        &crate::smtp::Timeouts::for_tests(Duration::from_secs(3)),
        &crate::smtp::TlsSource::Test(tls.client.clone()),
    )
    .await
    .unwrap();
}
async fn lifecycle(rotating: bool) {
    let tls = crate::net::test_support::TestTls::for_names(&["localhost"]);
    let (ip, sp, imap_task, smtp_task) = protocol_servers(tls.server.clone()).await;
    let (port, count, oauth_task) = oauth_server(rotating).await;
    let registration = ProviderRegistration {
        id: if rotating {
            "fixture-rotating"
        } else {
            "fixture-stable"
        }
        .into(),
        issuer: "fixture-issuer".into(),
        client_id: "public-client".into(),
        registration_id: uuid::Uuid::new_v4().to_string(),
        mail_policy: MailEndpointPolicy {
            imap: MailProtocolPolicy {
                host: "localhost".into(),
                port: ip,
                tls: crate::TlsMode::Implicit,
                login: MailLoginPolicy::Fixed("lin".into()),
            },
            smtp: MailProtocolPolicy {
                host: "localhost".into(),
                port: sp,
                tls: crate::TlsMode::Implicit,
                login: MailLoginPolicy::Fixed("lin".into()),
            },
        },
        authorization_endpoint: Url::parse(&format!("http://127.0.0.1:{port}/authorize")).unwrap(),
        token_endpoint: Url::parse(&format!("http://127.0.0.1:{port}/token")).unwrap(),
        scopes: vec!["mail".into()],
        resource: Some("urn:mail".into()),
        allowed_targets: vec![],
        redirect_path: "/callback".into(),
        extra_authorize_params: vec![],
        login_hint: false,
    };
    let mut config = sync_plan::tests::config(serde_json::json!({"kind":"direct"}));
    config["imap"]["host"] = "localhost".into();
    config["imap"]["port"] = ip.into();
    config["smtp"]["host"] = "localhost".into();
    config["smtp"]["port"] = sp.into();
    config["authentication"] = serde_json::json!({"kind":"oauth2","configuration":{"providerId":registration.id,"issuer":registration.issuer,"clientRegistrationId":registration.registration_id,"authorizationSubject":null,"targetMailboxResource":"urn:mail"}});
    let request = serde_json::from_value(
        serde_json::json!({"requestId":uuid::Uuid::new_v4().to_string(),"config":config}),
    )
    .unwrap();
    let coordinator = oauth_sessions::SessionCoordinator::default();
    let start = coordinator
        .begin(
            registration.clone(),
            Arc::new(FixtureIdentity(port)),
            NetRoute::default(),
            request,
        )
        .await
        .unwrap();
    let authorization = Url::parse(&start.authorization_url).unwrap();
    let params: std::collections::HashMap<_, _> =
        authorization.query_pairs().into_owned().collect();
    assert_eq!(params["resource"], "urn:mail");
    let mut callback = Url::parse(&params["redirect_uri"]).unwrap();
    callback.query_pairs_mut().extend_pairs([
        ("state", params["state"].as_str()),
        ("code", "fixture-code"),
    ]);
    let mut browser = tokio::net::TcpStream::connect(("127.0.0.1", callback.port().unwrap()))
        .await
        .unwrap();
    browser
        .write_all(
            format!(
                "GET {}?{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
                callback.path(),
                callback.query().unwrap(),
                callback.port().unwrap()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let completed = coordinator.finish(&start.session_id).await.unwrap();
    assert!(coordinator.finish(&start.session_id).await.is_err());
    let directory = tempfile::TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let mut service = AccountService::open(directory.path(), secrets.clone()).unwrap();
    let account = service
        .create_oauth(
            completed.request,
            completed.authorization,
            completed.registration,
        )
        .unwrap();
    authenticate_plans(&service, &account.id, &tls).await;
    let snapshot = service.storage.load(&account.id).unwrap();
    let key = snapshot.profiles[0].secret_ref.clone();
    drop(service);
    let mut tokens = TokenSet::decode_secret(&secrets.get(&key).unwrap().unwrap()).unwrap();
    tokens.expires_at = 0;
    secrets.set(&key, &tokens.encode_secret().unwrap()).unwrap();
    let mut reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    reopened.trust_test_oauth_policy(&registration);
    authenticate_plans(&reopened, &account.id, &tls).await;
    let saved = TokenSet::decode_secret(&secrets.get(&key).unwrap().unwrap()).unwrap();
    assert_eq!(
        saved.refresh_token.unwrap().expose_secret(),
        if rotating {
            "rotated-refresh"
        } else {
            "stable-refresh"
        }
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    oauth_task.await.unwrap();
    imap_task.await.unwrap();
    smtp_task.await.unwrap();
}
#[tokio::test]
async fn stable_refresh_fixture_full_lifecycle() {
    lifecycle(false).await;
}
#[tokio::test]
async fn rotating_refresh_fixture_full_lifecycle() {
    lifecycle(true).await;
}

fn pending_fixture(port: u16, email: &str) -> (ProviderRegistration, CreateAccountRequest) {
    let registration = ProviderRegistration {
        id: "negative-fixture".into(),
        issuer: "fixture-issuer".into(),
        client_id: "fixture-client".into(),
        registration_id: "fixture-registration".into(),
        mail_policy: MailEndpointPolicy {
            imap: MailProtocolPolicy {
                host: "imap.example.com".into(),
                port: 993,
                tls: crate::TlsMode::Implicit,
                login: MailLoginPolicy::Fixed("lin".into()),
            },
            smtp: MailProtocolPolicy {
                host: "smtp.example.com".into(),
                port: 465,
                tls: crate::TlsMode::Implicit,
                login: MailLoginPolicy::Fixed("lin".into()),
            },
        },
        authorization_endpoint: Url::parse(&format!("http://127.0.0.1:{port}/authorize")).unwrap(),
        token_endpoint: Url::parse(&format!("http://127.0.0.1:{port}/token")).unwrap(),
        scopes: vec!["mail".into()],
        resource: Some("urn:mail".into()),
        allowed_targets: vec![],
        redirect_path: "/callback".into(),
        extra_authorize_params: vec![],
        login_hint: false,
    };
    let mut config = sync_plan::tests::config(serde_json::json!({"kind":"direct"}));
    config["emailAddress"] = email.into();
    config["authentication"] = serde_json::json!({"kind":"oauth2","configuration":{"providerId":registration.id,"issuer":registration.issuer,"clientRegistrationId":registration.registration_id,"authorizationSubject":null,"targetMailboxResource":"urn:mail"}});
    (
        registration,
        serde_json::from_value(
            serde_json::json!({"requestId":uuid::Uuid::new_v4().to_string(),"config":config}),
        )
        .unwrap(),
    )
}
#[tokio::test]
async fn cancelled_authorization_closes_listener_and_cannot_be_consumed() {
    let (registration, request) = pending_fixture(9, "lin@example.com");
    let coordinator = oauth_sessions::SessionCoordinator::default();
    let start = coordinator
        .begin(
            registration,
            Arc::new(FixtureIdentity(9)),
            NetRoute::default(),
            request,
        )
        .await
        .unwrap();
    let authorization = Url::parse(&start.authorization_url).unwrap();
    let redirect = Url::parse(
        &authorization
            .query_pairs()
            .find(|(k, _)| k == "redirect_uri")
            .unwrap()
            .1,
    )
    .unwrap();
    coordinator.cancel(&start.session_id).unwrap();
    assert!(coordinator.finish(&start.session_id).await.is_err());
    tokio::task::yield_now().await;
    assert!(
        tokio::net::TcpListener::bind(("127.0.0.1", redirect.port().unwrap()))
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn wrong_verified_identity_never_produces_consumable_authorization() {
    let (port, count, server) = oauth_server(false).await;
    let (registration, request) = pending_fixture(port, "other@example.com");
    let coordinator = oauth_sessions::SessionCoordinator::default();
    let start = coordinator
        .begin(
            registration,
            Arc::new(FixtureIdentity(port)),
            NetRoute::default(),
            request,
        )
        .await
        .unwrap();
    let authorization = Url::parse(&start.authorization_url).unwrap();
    let params: std::collections::HashMap<_, _> =
        authorization.query_pairs().into_owned().collect();
    let redirect = Url::parse(&params["redirect_uri"]).unwrap();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", redirect.port().unwrap()))
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "GET {}?state={}&code=fixture-code HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
                redirect.path(),
                params["state"],
                redirect.port().unwrap()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    assert!(matches!(
        coordinator.finish(&start.session_id).await,
        Err(OAuthError::InvalidResponse)
    ));
    assert!(coordinator.finish(&start.session_id).await.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    server.abort();
}

/// One refresh-token grant returning `access`; yields the request body.
async fn refresh_server(access: &'static str) -> (u16, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        let mut len = 0;
        loop {
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                len = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; len];
        stream.read_exact(&mut body).await.unwrap();
        // No `expires_in` and no `scope`: both are optional in RFC 6749 §5.1.
        let response = format!(r#"{{"access_token":"{access}","token_type":"Bearer"}}"#);
        stream.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).as_bytes()).await.unwrap();
        String::from_utf8(body).unwrap()
    });
    (port, task)
}

/// A grant whose cached access token is still fresh locally but has been
/// revoked by the provider.
fn revoked_manager(token_port: u16, imap: u16, smtp: u16) -> Arc<TokenManager> {
    let registration = ProviderRegistration {
        id: "revoked-fixture".into(),
        issuer: "fixture-issuer".into(),
        client_id: "public-client".into(),
        registration_id: "revoked-registration".into(),
        mail_policy: MailEndpointPolicy {
            imap: MailProtocolPolicy {
                host: "localhost".into(),
                port: imap,
                tls: crate::TlsMode::Implicit,
                login: MailLoginPolicy::Fixed("lin".into()),
            },
            smtp: MailProtocolPolicy {
                host: "localhost".into(),
                port: smtp,
                tls: crate::TlsMode::Implicit,
                login: MailLoginPolicy::Fixed("lin".into()),
            },
        },
        authorization_endpoint: Url::parse(&format!("http://127.0.0.1:{token_port}/authorize"))
            .unwrap(),
        token_endpoint: Url::parse(&format!("http://127.0.0.1:{token_port}/token")).unwrap(),
        scopes: vec!["mail".into()],
        resource: None,
        allowed_targets: vec![],
        redirect_path: "/callback".into(),
        extra_authorize_params: vec![],
        login_hint: false,
    };
    let mut manager = TokenManager::new(
        registration,
        Arc::new(MemorySecrets::default()),
        "revoked-grant".into(),
    )
    .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    manager
        .restore(TokenSet {
            access_token: SecretValue::new("revoked-access".into()),
            refresh_token: Some(SecretValue::new("still-valid-refresh".into())),
            expires_at: now + 3600,
            target: Some(crate::auth::TokenTarget::new(None, vec!["mail".into()])),
            scopes: vec!["mail".into()],
        })
        .unwrap();
    Arc::new(manager)
}

/// SMTP server that rejects every bearer except `accepted`.
async fn rejecting_smtp(
    tls: Arc<rustls::ServerConfig>,
    attempts: Vec<&'static str>,
    accepted: Option<&'static str>,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        for token in attempts {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_rustls::TlsAcceptor::from(tls.clone())
                .accept(stream)
                .await
                .unwrap();
            let mut stream = BufReader::new(stream);
            stream
                .get_mut()
                .write_all(b"220 fixture\r\n")
                .await
                .unwrap();
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            stream
                .get_mut()
                .write_all(b"250-fixture\r\n250 AUTH XOAUTH2\r\n")
                .await
                .unwrap();
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            let encoded = line.trim().strip_prefix("AUTH XOAUTH2 ").unwrap();
            assert_eq!(
                STANDARD.decode(encoded).unwrap(),
                format!("user=lin\x01auth=Bearer {token}\x01\x01").as_bytes()
            );
            if Some(token) == accepted {
                stream.get_mut().write_all(b"235 ok\r\n").await.unwrap();
                line.clear();
                stream.read_line(&mut line).await.unwrap();
                assert_eq!(line, "QUIT\r\n");
                stream.get_mut().write_all(b"221 bye\r\n").await.unwrap();
            } else {
                stream
                    .get_mut()
                    .write_all(b"535 5.7.8 token revoked\r\n")
                    .await
                    .unwrap();
            }
        }
    });
    (port, task)
}

#[tokio::test]
async fn smtp_rejection_of_a_locally_fresh_token_refreshes_once_and_retries() {
    let tls = crate::net::test_support::TestTls::for_names(&["localhost"]);
    let (token_port, refresh) = refresh_server("replacement-access").await;
    let (smtp, server) = rejecting_smtp(
        tls.server.clone(),
        vec!["revoked-access", "replacement-access"],
        Some("replacement-access"),
    )
    .await;
    let manager = revoked_manager(token_port, 1, smtp);
    let login = crate::smtp::SmtpLogin {
        host: "localhost".into(),
        port: smtp,
        tls: crate::TlsMode::Implicit,
        username: "lin".into(),
        credential: ProtocolCredential::ManagedOAuth2(manager.clone()),
    };
    crate::smtp::verify_connection(
        &NetRoute::default(),
        &login,
        &crate::smtp::Timeouts::for_tests(Duration::from_secs(3)),
        &crate::smtp::TlsSource::Test(tls.client.clone()),
    )
    .await
    .unwrap();
    assert!(
        refresh
            .await
            .unwrap()
            .contains("refresh_token=still-valid-refresh")
    );
    server.await.unwrap();
    assert_eq!(
        manager.cached_token().unwrap().expose_secret(),
        "replacement-access"
    );
    assert!(!manager.needs_reauthorization());
}

#[tokio::test]
async fn smtp_rejection_of_a_refreshed_token_is_reported_without_looping() {
    let tls = crate::net::test_support::TestTls::for_names(&["localhost"]);
    let (token_port, refresh) = refresh_server("also-rejected").await;
    let (smtp, server) = rejecting_smtp(
        tls.server.clone(),
        vec!["revoked-access", "also-rejected"],
        None,
    )
    .await;
    let manager = revoked_manager(token_port, 1, smtp);
    let login = crate::smtp::SmtpLogin {
        host: "localhost".into(),
        port: smtp,
        tls: crate::TlsMode::Implicit,
        username: "lin".into(),
        credential: ProtocolCredential::ManagedOAuth2(manager),
    };
    let outcome = crate::smtp::verify_connection(
        &NetRoute::default(),
        &login,
        &crate::smtp::Timeouts::for_tests(Duration::from_secs(3)),
        &crate::smtp::TlsSource::Test(tls.client.clone()),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            &outcome,
            crate::smtp::SmtpOutcome::Permanent(failure)
                if failure.reason == crate::smtp::FailureReason::SignInRequired
        ),
        "{outcome:?}"
    );
    refresh.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn imap_rejection_of_a_locally_fresh_token_refreshes_once_and_retries() {
    let tls = crate::net::test_support::TestTls::for_names(&["localhost"]);
    let (token_port, refresh) = refresh_server("replacement-access").await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let imap = listener.local_addr().unwrap().port();
    let server_tls = tls.server.clone();
    let server = tokio::spawn(async move {
        for token in ["revoked-access", "replacement-access"] {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_rustls::TlsAcceptor::from(server_tls.clone())
                .accept(stream)
                .await
                .unwrap();
            let mut stream = BufReader::new(stream);
            stream.get_mut().write_all(b"* OK ready\r\n").await.unwrap();
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap().to_owned();
            assert!(line.contains("AUTHENTICATE XOAUTH2"));
            stream.get_mut().write_all(b"+ \r\n").await.unwrap();
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            assert_eq!(
                STANDARD.decode(line.trim()).unwrap(),
                format!("user=lin\x01auth=Bearer {token}\x01\x01").as_bytes()
            );
            if token == "revoked-access" {
                stream
                    .get_mut()
                    .write_all(format!("{tag} NO [AUTHENTICATIONFAILED] revoked\r\n").as_bytes())
                    .await
                    .unwrap();
                continue;
            }
            stream
                .get_mut()
                .write_all(format!("{tag} OK authenticated\r\n").as_bytes())
                .await
                .unwrap();
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap();
            stream
                .get_mut()
                .write_all(format!("* CAPABILITY IMAP4rev1\r\n{tag} OK done\r\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let manager = revoked_manager(token_port, imap, 1);
    let login = crate::imap::ImapLogin {
        host: "localhost".into(),
        port: imap,
        tls: crate::TlsMode::Implicit,
        username: "lin".into(),
        credential: ProtocolCredential::ManagedOAuth2(manager.clone()),
    };
    let _session =
        crate::imap::test_network_session(&NetRoute::default(), &login, tls.client.clone())
            .await
            .unwrap();
    assert!(
        refresh
            .await
            .unwrap()
            .contains("refresh_token=still-valid-refresh")
    );
    server.await.unwrap();
    assert_eq!(
        manager.cached_token().unwrap().expose_secret(),
        "replacement-access"
    );
}
