use super::*;
use crate::accounts::sync_plan::tests::config;
use crate::secrets::{SecretStore, SecretStoreError, SecretValue};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Default)]
struct CountingSecrets(AtomicUsize);
impl SecretStore for CountingSecrets {
    fn get(&self, _: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
    fn set(&self, _: &str, _: &SecretValue) -> Result<(), SecretStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn delete(&self, _: &str) -> Result<(), SecretStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn setup() -> (TempDir, MailxCore, Arc<CountingSecrets>) {
    let directory = TempDir::new().unwrap();
    let secrets = Arc::new(CountingSecrets::default());
    let core = MailxCore::open(
        directory.path(),
        secrets.clone(),
        tokio::runtime::Handle::current(),
        Arc::new(|_: &crate::CoreEvent| {}),
    )
    .unwrap();
    (directory, core, secrets)
}

fn request(imap_port: u16, smtp_port: u16) -> VerifyDraftConnection {
    let mut value = config(json!({"kind":"direct"}));
    value.as_object_mut().unwrap().remove("displayName");
    for (protocol, port) in [("imap", imap_port), ("smtp", smtp_port)] {
        value[protocol]["host"] = json!("127.0.0.1");
        value[protocol]["port"] = json!(port);
        value[protocol]["tls"] = json!("startTls");
    }
    VerifyDraftConnection {
        verification_id: Uuid::new_v4().to_string(),
        connection: serde_json::from_value(value).unwrap(),
    }
}

#[tokio::test]
async fn probes_are_independent_and_do_not_access_the_service_or_secret_store() {
    let imap_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let smtp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let smtp_port = smtp_listener.local_addr().unwrap().port();
    let (_directory, core, secrets) = setup();
    let attempt = request(imap_listener.local_addr().unwrap().port(), smtp_port);
    let id = attempt.verification_id.clone();
    let service = core.service.clone();
    let (held, acquired) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _lock = service.lock().unwrap();
        held.send(()).unwrap();
        let _ = released.recv();
    });
    acquired.await.unwrap();
    let smtp_peer = tokio::spawn(async move {
        // A controlled EOF produces the same network failure without relying on
        // platform-specific connection-refusal timing or reusing a closed port.
        let (stream, _) = smtp_listener.accept().await.unwrap();
        drop(stream);
    });
    let peer = tokio::spawn(async move {
        let (stream, _) = imap_listener.accept().await.unwrap();
        let mut peer = BufReader::new(stream);
        peer.get_mut().write_all(b"* OK test\r\n").await.unwrap();
        let mut line = String::new();
        peer.read_line(&mut line).await.unwrap();
        let tag = line.split_once(' ').unwrap().0;
        assert!(line.ends_with(" STARTTLS\r\n"), "{line}");
        peer.get_mut()
            .write_all(format!("{tag} NO no TLS private-mail-secret\r\n").as_bytes())
            .await
            .unwrap();
        let mut remaining = Vec::new();
        peer.read_to_end(&mut remaining).await.unwrap();
        assert!(remaining.is_empty(), "no authentication without TLS");
    });
    // This is a deadlock watchdog, not a performance assertion. The service lock
    // stays held until both probes finish, including on a busy CI runner.
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        core.verify_draft_connection(attempt),
    )
    .await;
    release.send(()).unwrap();
    holder.join().unwrap();
    let report = result.unwrap().unwrap();
    peer.await.unwrap();
    smtp_peer.await.unwrap();
    assert_eq!(report.verification_id, id);
    assert_eq!(
        report.imap.code,
        Some(ConnectionVerificationCode::Unsupported)
    );
    assert_eq!(report.smtp.code, Some(ConnectionVerificationCode::Network));
    assert!(core.list_accounts().unwrap().is_empty());
    assert!(core.sync_states().is_empty());
    assert_eq!(secrets.0.load(Ordering::SeqCst), 0);
    let result = serde_json::to_string(&report).unwrap();
    for secret in [
        "private-mail-secret",
        "mail-secret",
        "127.0.0.1",
        "lin@example.com",
    ] {
        assert!(!result.contains(secret));
    }
}

#[tokio::test]
async fn cancellation_before_registration_prevents_preparation_and_network() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_directory, core, secrets) = setup();
    let mut attempt = request(port, port);
    // Even invalid pending credentials are not prepared after an early abort.
    attempt.connection.imap.port = 0;
    core.cancel_draft_verification(CancelDraftVerification {
        verification_id: attempt.verification_id.clone(),
    })
    .unwrap();
    let report = core.verify_draft_connection(attempt).await.unwrap();
    assert_eq!(
        report.imap.code,
        Some(ConnectionVerificationCode::Cancelled)
    );
    assert_eq!(
        report.smtp.code,
        Some(ConnectionVerificationCode::Cancelled)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    assert_eq!(secrets.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn concurrent_drafts_conflict_and_an_old_abort_cannot_cancel_the_new_attempt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_directory, core, _) = setup();
    let attempt = request(port, port);
    let id = attempt.verification_id.clone();
    let running_core = core.clone();
    let running = tokio::spawn(async move { running_core.verify_draft_connection(attempt).await });
    let (mut socket, _) = listener.accept().await.unwrap();
    assert_eq!(
        core.verify_draft_connection(request(port, port))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    core.cancel_draft_verification(CancelDraftVerification {
        verification_id: Uuid::new_v4().to_string(),
    })
    .unwrap();
    assert!(!running.is_finished());
    core.cancel_draft_verification(CancelDraftVerification {
        verification_id: id,
    })
    .unwrap();
    let report = tokio::time::timeout(Duration::from_secs(1), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        report.imap.code,
        Some(ConnectionVerificationCode::Cancelled)
    );
    assert_eq!(
        report.smtp.code,
        Some(ConnectionVerificationCode::Cancelled)
    );
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(bytes.is_empty());
    // Dropping the cancelled future frees the one draft slot immediately.
    let (_registration, _) = core.draft_verification.start(DRAFT_SLOT, "next").unwrap();
}

#[tokio::test]
async fn a_failed_custom_proxy_never_connects_directly() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination_port = destination.local_addr().unwrap().port();
    let proxy_port = proxy.local_addr().unwrap().port();
    let (_directory, core, secrets) = setup();
    let mut attempt = request(destination_port, destination_port);
    attempt.connection.route = serde_json::from_value(json!({
        "kind":"custom", "proxyKind":"httpConnect", "host":"127.0.0.1", "port":proxy_port, "dns":"proxy",
        "authentication":{"username":"proxy-user","password":{"action":"replace","value":"proxy-secret"}}
    })).unwrap();
    let proxy_peer = tokio::spawn(async move {
        for _ in 0..2 {
            let (stream, _) = proxy.accept().await.unwrap();
            let mut peer = BufReader::new(stream);
            let mut request = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(peer.read_line(&mut line).await.unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            assert!(request.starts_with(&format!(
                "CONNECT 127.0.0.1:{destination_port} HTTP/1.1\r\n"
            )));
            assert!(request.contains("Proxy-Authorization: Basic "));
            peer.get_mut()
                .write_all(b"HTTP/1.1 502 private-proxy-secret\r\n\r\n")
                .await
                .unwrap();
        }
    });
    let report = tokio::time::timeout(
        Duration::from_secs(2),
        core.verify_draft_connection(attempt),
    )
    .await
    .unwrap()
    .unwrap();
    proxy_peer.await.unwrap();
    assert_eq!(report.imap.code, Some(ConnectionVerificationCode::Proxy));
    assert_eq!(report.smtp.code, Some(ConnectionVerificationCode::Proxy));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), destination.accept())
            .await
            .is_err()
    );
    assert_eq!(secrets.0.load(Ordering::SeqCst), 0);
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("proxy-secret")
    );
}

#[tokio::test]
async fn the_total_budget_cancels_both_protocols_and_releases_the_slot() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_directory, core, _) = setup();
    let attempt = request(port, port);
    let running_core = core.clone();
    let running = tokio::spawn(async move { running_core.verify_draft_connection(attempt).await });
    let (_first, _) = listener.accept().await.unwrap();
    let (_second, _) = listener.accept().await.unwrap();
    tokio::time::pause();
    tokio::time::advance(BUDGET).await;
    let report = running.await.unwrap().unwrap();
    assert_eq!(report.imap.code, Some(ConnectionVerificationCode::Timeout));
    assert_eq!(report.smtp.code, Some(ConnectionVerificationCode::Timeout));
    let (_registration, _) = core.draft_verification.start(DRAFT_SLOT, "next").unwrap();
}

#[tokio::test]
async fn invalid_new_credentials_return_fixed_validation_without_a_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_directory, core, secrets) = setup();
    let mut attempt = request(port, port);
    attempt.connection.authentication = serde_json::from_value(json!({
        "kind":"password", "mode":"shared", "password":{"action":"keep"}
    }))
    .unwrap();
    assert_eq!(
        core.verify_draft_connection(attempt).await.unwrap_err(),
        AppError::validation("authentication")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    assert_eq!(secrets.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_shared_imap_probe_only_authenticates_checks_liveness_and_logs_out() {
    let attempt = request(993, 465);
    let plan = draft_connection::prepare(attempt.connection).unwrap();
    let (client, server) = tokio::io::duplex(4096);
    let peer = tokio::spawn(async move {
        let mut peer = BufReader::new(server);
        peer.get_mut()
            .write_all(b"* OK synthetic\r\n")
            .await
            .unwrap();
        for (expected, response) in [
            ("LOGIN \"lin\" \"mail-secret\"", ""),
            ("CAPABILITY", "* CAPABILITY IMAP4rev1\r\n"),
            ("NOOP", ""),
            ("LOGOUT", "* BYE synthetic\r\n"),
        ] {
            let mut line = String::new();
            peer.read_line(&mut line).await.unwrap();
            let (tag, command) = line.trim_end().split_once(' ').unwrap();
            assert_eq!(
                command, expected,
                "mailbox and message commands are forbidden"
            );
            peer.get_mut()
                .write_all(format!("{response}{tag} OK done\r\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let session = imap::test_body_session(Box::new(client), &plan.imap)
        .await
        .unwrap();
    let result = verify_authenticated_imap(session).await;
    assert_eq!(result, ProtocolVerification::verified());
    peer.await.unwrap();
}

#[test]
fn cancellation_while_draft_preparation_is_queued_never_opens_a_socket() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (_directory, core, secrets) = setup();
        let (entered, occupied) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            released.recv().unwrap();
        });
        occupied.await.unwrap();
        let attempt = request(port, port);
        let id = attempt.verification_id.clone();
        let running_core = core.clone();
        let running =
            tokio::spawn(async move { running_core.verify_draft_connection(attempt).await });
        while core.draft_verification.0.lock().unwrap().flights.is_empty() {
            tokio::task::yield_now().await;
        }
        core.cancel_draft_verification(CancelDraftVerification {
            verification_id: id,
        })
        .unwrap();
        let report = tokio::time::timeout(Duration::from_secs(1), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            report.imap.code,
            Some(ConnectionVerificationCode::Cancelled)
        );
        assert_eq!(
            report.smtp.code,
            Some(ConnectionVerificationCode::Cancelled)
        );
        release.send(()).unwrap();
        blocker.await.unwrap();
        // Drain the old preparation before checking that it did not connect.
        tokio::task::spawn_blocking(|| ()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        assert_eq!(secrets.0.load(Ordering::SeqCst), 0);
    });
}
