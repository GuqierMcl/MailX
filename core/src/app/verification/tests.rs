use super::*;
use crate::accounts::sync_plan::tests::{config, connection_update};
use crate::secrets::test_support::MemorySecrets;
use crate::secrets::{SecretStore, SecretStoreError, SecretValue};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tokio::net::TcpListener;
use uuid::Uuid;

fn request(account: &crate::Account) -> VerifyAccountConnection {
    VerifyAccountConnection {
        account_id: account.id.clone(),
        expected_revision: account.revision,
        verification_id: Uuid::new_v4().to_string(),
    }
}
fn setup(port: u16) -> (TempDir, MailxCore, crate::Account) {
    setup_with_secrets(port, Arc::new(MemorySecrets::default()))
}

fn setup_with_secrets(
    port: u16,
    secrets: Arc<dyn SecretStore>,
) -> (TempDir, MailxCore, crate::Account) {
    let directory = TempDir::new().unwrap();
    let core = MailxCore::open(
        directory.path(),
        secrets,
        tokio::runtime::Handle::current(),
        Arc::new(|_: &crate::CoreEvent| {}),
    )
    .unwrap();
    let mut value = config(json!({"kind":"direct"}));
    for protocol in ["imap", "smtp"] {
        value[protocol]["host"] = json!("127.0.0.1");
        value[protocol]["port"] = json!(port);
    }
    // Avoid starting automatic sync: only this explicitly requested diagnostic connects.
    let account = core
        .service
        .lock()
        .unwrap()
        .create(
            serde_json::from_value(json!({"requestId":Uuid::new_v4().to_string(), "config":value}))
                .unwrap(),
        )
        .unwrap();
    (directory, core, account)
}

#[tokio::test]
async fn stale_revision_never_opens_a_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_directory, core, account) = setup(listener.local_addr().unwrap().port());
    let mut stale = request(&account);
    stale.expected_revision += 1;
    assert_eq!(
        core.verify_account_connection(stale)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_releases_sockets_and_storage_is_unlocked_during_network() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_directory, core, account) = setup(listener.local_addr().unwrap().port());
    let attempt = request(&account);
    let task_core = core.clone();
    let task_attempt = attempt.clone();
    let running =
        tokio::spawn(async move { task_core.verify_account_connection(task_attempt).await });
    let (mut socket, _) = listener.accept().await.unwrap();
    let _storage = core
        .service
        .try_lock()
        .expect("network never holds the store lock");
    drop(_storage);
    assert_eq!(
        core.verify_account_connection(request(&account))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    core.cancel_account_connection_verification(CancelConnectionVerification {
        account_id: account.id.clone(),
        verification_id: attempt.verification_id,
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
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancelled_before_registration_does_not_connect_and_old_cancel_cannot_stop_a_new_attempt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_directory, core, account) = setup(listener.local_addr().unwrap().port());
    let attempt = request(&account);
    core.cancel_account_connection_verification(CancelConnectionVerification {
        account_id: account.id.clone(),
        verification_id: attempt.verification_id.clone(),
    })
    .unwrap();
    assert_eq!(
        core.verify_account_connection(attempt)
            .await
            .unwrap()
            .imap
            .code,
        Some(ConnectionVerificationCode::Cancelled)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    let registry = Arc::new(VerificationRegistry::default());
    let (_active, receiver) = registry.start(&account.id, "new").unwrap();
    registry.cancel(&account.id, "old").unwrap();
    assert_eq!(*receiver.borrow(), VerificationControl::Active);
}

#[tokio::test]
async fn an_updated_connection_invalidates_in_flight_results() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_directory, core, account) = setup(listener.local_addr().unwrap().port());
    let attempt = request(&account);
    let task_core = core.clone();
    let task_attempt = attempt.clone();
    let running =
        tokio::spawn(async move { task_core.verify_account_connection(task_attempt).await });
    let (_socket, _) = listener.accept().await.unwrap();
    let mut value = config(json!({"kind":"direct"}));
    value["imap"]["port"] = json!(listener.local_addr().unwrap().port());
    core.service
        .lock()
        .unwrap()
        .update_connection(connection_update(&account.id, account.revision, value))
        .unwrap();
    core.verification.invalidate(&account.id);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test(start_paused = true)]
async fn stalled_operations_have_a_total_deadline() {
    let (_cancel, receiver) = watch::channel(VerificationControl::Active);
    let started = tokio::time::Instant::now();
    let result = bounded(started + BUDGET, receiver, std::future::pending::<()>()).await;
    assert_eq!(result, Err(VerificationControl::TimedOut));
    assert_eq!(started.elapsed(), BUDGET);
}

#[tokio::test(start_paused = true)]
async fn preparation_and_final_validation_share_the_network_deadline() {
    let (_cancel, receiver) = watch::channel(VerificationControl::Active);
    let started = tokio::time::Instant::now();
    let result = bounded(started + BUDGET, receiver, async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
    })
    .await;
    assert_eq!(result, Err(VerificationControl::TimedOut));
    assert_eq!(started.elapsed(), BUDGET);
}

#[tokio::test]
async fn cancellation_while_waiting_for_storage_releases_the_registration() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_directory, core, account) = setup(listener.local_addr().unwrap().port());
    let service = core.service.clone();
    let (held, acquired) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _lock = service.lock().unwrap();
        held.send(()).unwrap();
        let _ = released.recv();
    });
    acquired.await.unwrap();
    let attempt = request(&account);
    let task_core = core.clone();
    let task_attempt = attempt.clone();
    let running =
        tokio::spawn(async move { task_core.verify_account_connection(task_attempt).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if core
                .verification
                .0
                .lock()
                .unwrap()
                .flights
                .contains_key(&account.id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    core.cancel_account_connection_verification(CancelConnectionVerification {
        account_id: account.id.clone(),
        verification_id: attempt.verification_id,
    })
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(1), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        result.imap.code,
        Some(ConnectionVerificationCode::Cancelled)
    );
    let (_registration, _) = core.verification.start(&account.id, "next").unwrap();
    release.send(()).unwrap();
    holder.join().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

type SecretGate = (
    tokio::sync::oneshot::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);
#[derive(Default)]
struct GatedSecrets {
    inner: MemorySecrets,
    reads: AtomicUsize,
    gate: Mutex<Option<SecretGate>>,
}
impl SecretStore for GatedSecrets {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some((entered, release)) = self.gate.lock().unwrap().take() {
            let _ = entered.send(());
            let _ = release.recv();
        }
        self.inner.get(key)
    }
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        self.inner.delete(key)
    }
}

#[tokio::test]
async fn version_checks_do_not_read_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let secrets = Arc::new(GatedSecrets::default());
    let (_directory, core, account) = setup_with_secrets(port, secrets.clone());
    secrets.reads.store(0, Ordering::SeqCst);
    let report = core
        .verify_account_connection(request(&account))
        .await
        .unwrap();
    assert_eq!(report.revision, account.revision);
    // Only IMAP and SMTP plan construction read their shared credential. The
    // initial and final revision checks consult durable metadata alone.
    assert_eq!(secrets.reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancellation_during_a_secret_read_does_not_resolve_the_next_protocol() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let secrets = Arc::new(GatedSecrets::default());
    let (_directory, core, account) =
        setup_with_secrets(listener.local_addr().unwrap().port(), secrets.clone());
    secrets.reads.store(0, Ordering::SeqCst);
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    *secrets.gate.lock().unwrap() = Some((entered, released));
    let attempt = request(&account);
    let task_core = core.clone();
    let task_attempt = attempt.clone();
    let running =
        tokio::spawn(async move { task_core.verify_account_connection(task_attempt).await });
    started.await.unwrap();
    core.cancel_account_connection_verification(CancelConnectionVerification {
        account_id: account.id.clone(),
        verification_id: attempt.verification_id,
    })
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(1), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        result.imap.code,
        Some(ConnectionVerificationCode::Cancelled)
    );
    let (_registration, _) = core.verification.start(&account.id, "next").unwrap();
    release.send(()).unwrap();
    // Acquiring the store lock waits until the old secret read has unwound.
    let service = core.service.clone();
    tokio::task::spawn_blocking(move || drop(service.lock().unwrap()))
        .await
        .unwrap();
    assert_eq!(secrets.reads.load(Ordering::SeqCst), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[test]
fn external_errors_are_reduced_to_fixed_codes() {
    let marker = "private-account secret password raw-server-text";
    let imap = imap_failure(ImapError::Authentication(marker.into()));
    let smtp = smtp_failure(SmtpOutcome::Permanent(crate::smtp::PreDataFailure {
        reason: FailureReason::AuthRejected,
        code: Some(535),
        text: Some(marker.into()),
    }));
    for result in [imap, smtp] {
        let value = serde_json::to_string(&result).unwrap();
        assert!(!value.contains(marker));
        assert_eq!(
            result.code,
            Some(ConnectionVerificationCode::Authentication)
        );
    }
}

#[test]
fn provider_policy_diagnostic_is_a_fixed_code_with_no_server_text() {
    let report = imap_failure(ImapError::ProviderPolicy);
    assert_eq!(report.stage, ConnectionVerificationStage::Authentication);
    assert_eq!(
        report.code,
        Some(ConnectionVerificationCode::ProviderPolicy)
    );
    assert_eq!(
        serde_json::to_value(report).unwrap(),
        json!({
            "status": "failed", "stage": "authentication", "code": "providerPolicy"
        })
    );
}
