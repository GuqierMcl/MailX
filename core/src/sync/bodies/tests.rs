use super::*;

use crate::accounts::messages::SyncTarget;
use crate::accounts::sync_plan::tests::{config, connection_update};
use crate::accounts::types::Account;
use crate::imap::{FolderStatus, ImapLogin};
use crate::mail::{BodyState, EmailListQuery, FolderRole};
use crate::secrets::{SecretValue, test_support::MemorySecrets};
use crate::sync::{folders::FolderRecord, headers::MessageRecord};
use serde_json::json;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{Duration, timeout};

pub(crate) struct Fixture {
    pub(crate) _directory: TempDir,
    pub(crate) service: Arc<Mutex<AccountService>>,
    pub(crate) manager: Arc<BodyManager>,
    pub(crate) account: Account,
    pub(crate) target: SyncTarget,
    pub(crate) messages: Vec<String>,
    pub(crate) connections:
        mpsc::UnboundedReceiver<oneshot::Sender<Result<imap::Session, ImapError>>>,
}

pub(crate) fn create_account(service: &mut AccountService, label: &str) -> Account {
    let mut value = config(json!({"kind":"direct"}));
    value["emailAddress"] = json!(format!("{label}@example.test"));
    value["imap"]["username"] = json!(label);
    service
        .create(
            serde_json::from_value(json!({
                "requestId": Uuid::new_v4().to_string(), "config": value
            }))
            .unwrap(),
        )
        .unwrap()
}

pub(crate) fn seed(
    service: &mut AccountService,
    account: &Account,
    sizes: &[Option<u32>],
) -> (SyncTarget, Vec<String>) {
    service
        .store_folders(
            &account.id,
            &account.mailbox_source_id,
            &[FolderRecord {
                remote_name: "INBOX".into(),
                parent_remote_name: None,
                name: "INBOX".into(),
                delimiter: Some("/".into()),
                role: FolderRole::Inbox,
                selectable: true,
                attributes: Vec::new(),
                status: Some(FolderStatus {
                    messages: sizes.len() as u32,
                    unseen: sizes.len() as u32,
                    uid_next: Some(sizes.len() as u32 + 1),
                    uid_validity: Some(7),
                }),
            }],
        )
        .unwrap();
    let folder_id = service
        .sync_folders(&account.id, &account.mailbox_source_id)
        .unwrap()[0]
        .0
        .clone();
    let target = SyncTarget {
        account_id: account.id.clone(),
        mailbox_source_id: account.mailbox_source_id.clone(),
        folder_id: folder_id.clone(),
        uid_validity: 7,
    };
    service.ensure_generation(&target, "2026-09-19").unwrap();
    let records: Vec<_> = sizes
        .iter()
        .enumerate()
        .map(|(index, size)| MessageRecord {
            uid: index as u32 + 1,
            message_id_header: Some(format!("fixture-{index}@example.test")),
            subject: format!("Synthetic {index}"),
            from_name: None,
            from_address: "fixture@example.test".into(),
            to_addresses: Vec::new(),
            cc_addresses: Vec::new(),
            reply_to_addresses: Vec::new(),
            received_at: "2026-10-19T00:00:00.000Z".into(),
            sent_at: None,
            size: *size,
            is_read: false,
            is_flagged: false,
            flags: Vec::new(),
            has_attachment: false,
        })
        .collect();
    service
        .store_message_batch(&target, &records, records.len() as u32)
        .unwrap();
    let items = service
        .list_emails(&EmailListQuery {
            folder_id: Some(folder_id),
            ..EmailListQuery::default()
        })
        .unwrap()
        .items;
    let messages = (0..sizes.len())
        .map(|index| {
            items
                .iter()
                .find(|item| item.subject == format!("Synthetic {index}"))
                .unwrap()
                .id
                .clone()
        })
        .collect();
    (target, messages)
}

pub(crate) fn fixture(sizes: &[Option<u32>]) -> Fixture {
    let directory = TempDir::new().unwrap();
    let mut service =
        AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
    let account = create_account(&mut service, "first");
    let (target, messages) = seed(&mut service, &account, sizes);
    let service = Arc::new(Mutex::new(service));
    let mut manager = BodyManager::new(&service, Handle::current());
    let (connector, connections) = mpsc::unbounded_channel();
    Arc::get_mut(&mut manager.shared).unwrap().connector = Some(connector);
    let manager = Arc::new(manager);
    Fixture {
        _directory: directory,
        service,
        manager,
        account,
        target,
        messages,
        connections,
    }
}

pub(crate) fn fetch(fixture: &Fixture, index: usize) -> JoinHandle<Result<EmailDetail, AppError>> {
    let manager = fixture.manager.clone();
    let id = fixture.messages[index].clone();
    tokio::spawn(async move { manager.fetch(id).await })
}

pub(crate) async fn tagged_command(server: &mut BufReader<DuplexStream>, expected: &str) -> String {
    let mut command = String::new();
    server.read_line(&mut command).await.unwrap();
    let (tag, value) = command.trim_end().split_once(' ').unwrap();
    assert_eq!(value, expected);
    tag.into()
}

pub(crate) async fn session(
    uid: u32,
    validity: u32,
    raw: &'static [u8],
) -> (imap::Session, oneshot::Sender<()>, JoinHandle<()>) {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (release, released) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        server
            .get_mut()
            .write_all(b"* OK synthetic\r\n")
            .await
            .unwrap();
        let tag = tagged_command(&mut server, "LOGIN \"fixture\" \"synthetic-password\"").await;
        server
            .get_mut()
            .write_all(format!("{tag} OK\r\n").as_bytes())
            .await
            .unwrap();
        let tag = tagged_command(&mut server, "CAPABILITY").await;
        server
            .get_mut()
            .write_all(format!("* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n").as_bytes())
            .await
            .unwrap();
        let tag = tagged_command(&mut server, "EXAMINE \"INBOX\"").await;
        server
            .get_mut()
            .write_all(
                format!("* OK [UIDVALIDITY {validity}]\r\n{tag} OK [READ-ONLY]\r\n").as_bytes(),
            )
            .await
            .unwrap();
        if validity != 7 {
            return;
        }
        let tag = tagged_command(&mut server, &format!("UID FETCH {uid} (UID BODY.PEEK[])")).await;
        if released.await.is_err() {
            return;
        }
        let mut reply = format!("* 1 FETCH (UID {uid} BODY[] {{{}}}\r\n", raw.len()).into_bytes();
        reply.extend_from_slice(raw);
        reply.extend_from_slice(format!(")\r\n{tag} OK\r\n").as_bytes());
        let _ = server.get_mut().write_all(&reply).await;
    });
    let login = ImapLogin {
        host: "imap.example.test".into(),
        port: 993,
        tls: crate::accounts::types::TlsMode::Implicit,
        username: "fixture".into(),
        password: SecretValue::new("synthetic-password".into()),
    };
    let session = imap::test_body_session(Box::new(client), &login)
        .await
        .unwrap();
    (session, release, server)
}

pub(crate) const RAW: &[u8] =
    b"From: fixture@example.test\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nsynthetic body";

async fn finished(job: JoinHandle<Result<EmailDetail, AppError>>) -> Result<EmailDetail, AppError> {
    timeout(Duration::from_secs(3), job).await.unwrap().unwrap()
}

pub(crate) async fn connection(
    fixture: &mut Fixture,
) -> oneshot::Sender<Result<imap::Session, ImapError>> {
    timeout(Duration::from_secs(3), fixture.connections.recv())
        .await
        .unwrap()
        .unwrap()
}

pub(crate) async fn wait_until(mut condition: impl FnMut() -> bool) {
    timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn state(fixture: &Fixture, index: usize) -> BodyState {
    fixture
        .service
        .lock()
        .unwrap()
        .get_email(&fixture.messages[index])
        .unwrap()
        .summary
        .body_state
}

fn assert_ready(detail: &EmailDetail) {
    assert_eq!(detail.summary.body_state, BodyState::Ready);
    assert_eq!(
        detail.body.as_ref().unwrap().text.as_deref(),
        Some("synthetic body")
    );
    assert_eq!(detail.summary.preview, "synthetic body");
    assert!(!detail.summary.is_read);
    assert_eq!(detail.body_failure, None);
}

#[tokio::test]
async fn concurrent_requests_share_one_download_and_canceling_one_waiter_is_safe() {
    let mut fixture = fixture(&[None]);
    let first = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (session, release, server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(session)).is_ok());
    let second = fetch(&fixture, 0);
    wait_until(|| {
        fixture
            .manager
            .shared
            .flights
            .lock()
            .unwrap()
            .messages
            .values()
            .any(|flight| flight.result.receiver_count() == 2)
    })
    .await;
    first.abort();
    wait_until(|| {
        fixture
            .manager
            .shared
            .flights
            .lock()
            .unwrap()
            .messages
            .values()
            .any(|flight| flight.result.receiver_count() == 1)
    })
    .await;
    assert!(fixture.connections.try_recv().is_err());
    release.send(()).unwrap();
    assert_ready(&finished(second).await.unwrap());
    server.await.unwrap();
    assert_ready(
        &fixture
            .manager
            .fetch(fixture.messages[0].clone())
            .await
            .unwrap(),
    );
    assert!(fixture.connections.try_recv().is_err());
}

#[tokio::test]
async fn one_account_queues_bodies_while_another_account_can_download() {
    let mut fixture = fixture(&[None, None]);
    let (other, other_message) = {
        let mut service = fixture.service.lock().unwrap();
        let other = create_account(&mut service, "second");
        let (_, messages) = seed(&mut service, &other, &[None]);
        (other, messages[0].clone())
    };
    let first = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (first_session, first_release, first_server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(first_session)).is_ok());
    let second = fetch(&fixture, 1);
    wait_until(|| state(&fixture, 1) == BodyState::Fetching).await;
    let other_manager = fixture.manager.clone();
    let third = tokio::spawn(async move { other_manager.fetch(other_message).await });
    let other_request = connection(&mut fixture).await;
    let (other_session, other_release, other_server) = session(1, 7, RAW).await;
    assert!(other_request.send(Ok(other_session)).is_ok());
    other_release.send(()).unwrap();
    let other_result = finished(third).await.unwrap();
    assert_eq!(other_result.summary.account_id, other.id);
    assert_ready(&other_result);
    assert!(!first.is_finished());
    assert!(!second.is_finished());
    assert!(fixture.connections.try_recv().is_err());
    first_release.send(()).unwrap();
    assert_ready(&finished(first).await.unwrap());
    let request = connection(&mut fixture).await;
    let (second_session, second_release, second_server) = session(2, 7, RAW).await;
    assert!(request.send(Ok(second_session)).is_ok());
    second_release.send(()).unwrap();
    assert_ready(&finished(second).await.unwrap());
    first_server.await.unwrap();
    second_server.await.unwrap();
    other_server.await.unwrap();
}

#[tokio::test]
async fn account_removal_settles_pending_bodies_and_rejects_late_completion() {
    let mut fixture = fixture(&[None]);
    let pending = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let token = fixture.manager.shared.flights.lock().unwrap().messages[&fixture.messages[0]]
        .token
        .clone();
    {
        let mut service = fixture.service.lock().unwrap();
        service
            .remove(crate::RemoveAccountRequest {
                account_id: fixture.account.id.clone(),
                expected_revision: fixture.account.revision,
            })
            .unwrap();
        fixture.manager.retire(&fixture.account.id);
    }
    assert_eq!(
        finished(pending).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert!(request.send(Err(ImapError::Timeout)).is_err());
    assert_eq!(
        fail(&fixture.manager.shared, &token, BodyFailureCode::Network)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    complete(&fixture.manager.shared, &token, Err(internal()));
    assert_eq!(
        fixture
            .manager
            .fetch(fixture.messages[0].clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(fixture.connections.try_recv().is_err());
}

#[tokio::test]
async fn restart_settles_waiters_and_old_completion_cannot_replace_new_body() {
    let mut fixture = fixture(&[None]);
    let first = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (old_session, old_release, old_server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(old_session)).is_ok());
    let token = fixture.manager.shared.flights.lock().unwrap().messages[&fixture.messages[0]]
        .token
        .clone();
    {
        let mut service = fixture.service.lock().unwrap();
        fixture
            .manager
            .restart(&fixture.account.id, &mut service)
            .unwrap();
    }
    assert_eq!(finished(first).await.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(state(&fixture, 0), BodyState::Missing);
    let replacement = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (new_session, new_release, new_server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(new_session)).is_ok());
    new_release.send(()).unwrap();
    assert_ready(&finished(replacement).await.unwrap());
    // Represents a previously queued blocking continuation that outlived its
    // async worker: the generation check must reject its failure/commit action.
    assert_eq!(
        fail(&fixture.manager.shared, &token, BodyFailureCode::Network)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    complete(&fixture.manager.shared, &token, Err(internal()));
    assert_eq!(state(&fixture, 0), BodyState::Ready);
    drop(old_release);
    old_server.abort();
    new_server.await.unwrap();
}

#[tokio::test]
async fn known_oversize_is_persisted_without_opening_any_connection() {
    let mut fixture = fixture(&[Some(MAX_MESSAGE_BYTES as u32 + 1)]);
    let result = finished(fetch(&fixture, 0)).await.unwrap();
    assert_eq!(result.summary.body_state, BodyState::Failed);
    assert_eq!(result.body_failure, Some(BodyFailureCode::TooLarge));
    assert!(result.body.is_none());
    assert!(fixture.connections.try_recv().is_err());
}

#[tokio::test]
async fn connection_failures_persist_only_controlled_codes_and_retry_can_succeed() {
    let mut fixture = fixture(&[None]);
    for (error, expected) in [
        (
            ImapError::Authentication("private-marker".into()),
            BodyFailureCode::Authentication,
        ),
        (
            ImapError::Net(crate::net::NetError::Connect("private-marker".into())),
            BodyFailureCode::Network,
        ),
        (
            ImapError::Protocol("private-marker".into()),
            BodyFailureCode::Protocol,
        ),
        (ImapError::TooLarge, BodyFailureCode::TooLarge),
        (ImapError::Timeout, BodyFailureCode::Network),
    ] {
        let job = fetch(&fixture, 0);
        assert!(connection(&mut fixture).await.send(Err(error)).is_ok());
        let result = finished(job).await.unwrap();
        assert_eq!(result.summary.body_state, BodyState::Failed);
        assert_eq!(result.body_failure, Some(expected));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("private-marker")
        );
    }
    let job = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (session, release, server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    assert_ready(&finished(job).await.unwrap());
    server.await.unwrap();
}

#[tokio::test]
async fn uidvalidity_mismatch_is_stale_and_sends_no_body_fetch() {
    let mut fixture = fixture(&[None]);
    let job = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (session, release, server) = session(1, 999, RAW).await;
    assert!(request.send(Ok(session)).is_ok());
    let result = finished(job).await.unwrap();
    assert_eq!(result.summary.body_state, BodyState::Failed);
    assert_eq!(result.body_failure, Some(BodyFailureCode::Stale));
    drop(release);
    server.await.unwrap();
}

#[tokio::test]
async fn deleted_message_is_hidden_and_late_body_cannot_be_committed() {
    let mut fixture = fixture(&[None]);
    let job = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (session, release, server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(session)).is_ok());
    fixture
        .service
        .lock()
        .unwrap()
        .mark_deleted(&fixture.target, &[1])
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(finished(job).await.unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(
        fixture
            .manager
            .fetch(fixture.messages[0].clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(fixture.connections.try_recv().is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn protocol_read_of_missing_body_never_starts_a_download() {
    let mut fixture = fixture(&[None]);
    assert_eq!(
        fixture
            .manager
            .refresh(fixture.messages[0].clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(state(&fixture, 0), BodyState::Missing);
    assert!(fixture.connections.try_recv().is_err());
}

#[tokio::test]
async fn account_revision_change_restarts_old_work_without_reusing_its_session() {
    let mut fixture = fixture(&[None]);
    let first = fetch(&fixture, 0);
    let old_request = connection(&mut fixture).await;
    let old_token = fixture.manager.shared.flights.lock().unwrap().messages[&fixture.messages[0]]
        .token
        .clone();
    let updated = {
        let mut service = fixture.service.lock().unwrap();
        let mut value = config(json!({"kind":"direct"}));
        value["emailAddress"] = json!("first@example.test");
        value["imap"]["username"] = json!("first");
        value["smtp"]["host"] = json!("updated-smtp.example.test");
        value["authentication"]["password"] = json!({"action":"keep"});
        let updated = service
            .update_connection(connection_update(
                &fixture.account.id,
                fixture.account.revision,
                value,
            ))
            .unwrap();
        fixture
            .manager
            .restart(&fixture.account.id, &mut service)
            .unwrap();
        updated
    };
    assert_eq!(updated.mailbox_source_id, fixture.account.mailbox_source_id);
    assert!(updated.revision > fixture.account.revision);
    assert_eq!(finished(first).await.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(state(&fixture, 0), BodyState::Missing);
    assert!(old_request.send(Err(ImapError::Timeout)).is_err());
    let new_job = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (new_session, release, server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(new_session)).is_ok());
    release.send(()).unwrap();
    assert_ready(&finished(new_job).await.unwrap());
    assert_eq!(
        fail(
            &fixture.manager.shared,
            &old_token,
            BodyFailureCode::ProviderPolicy
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::NotFound
    );
    assert_eq!(state(&fixture, 0), BodyState::Ready);
    server.await.unwrap();
}

#[tokio::test]
async fn a_folder_becoming_noselect_hides_pending_body_and_rejects_late_commit() {
    let mut fixture = fixture(&[None]);
    let job = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (session, release, server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(session)).is_ok());
    fixture
        .service
        .lock()
        .unwrap()
        .store_folders(
            &fixture.account.id,
            &fixture.account.mailbox_source_id,
            &[FolderRecord {
                remote_name: "INBOX".into(),
                parent_remote_name: None,
                name: "INBOX".into(),
                delimiter: Some("/".into()),
                role: FolderRole::Other,
                selectable: false,
                attributes: vec!["\\Noselect".into()],
                status: None,
            }],
        )
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(finished(job).await.unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(
        fixture
            .manager
            .fetch(fixture.messages[0].clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(fixture.connections.try_recv().is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn transaction_failure_records_storage_failure_and_keeps_no_ready_body() {
    let mut fixture = fixture(&[None]);
    let job = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let db = rusqlite::Connection::open(fixture._directory.path().join("mailx.sqlite3")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_ready BEFORE UPDATE OF body_state ON messages
        WHEN NEW.body_state = 'ready' BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;",
    )
    .unwrap();
    let (session, release, server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    let result = finished(job).await.unwrap();
    assert_eq!(result.summary.body_state, BodyState::Failed);
    assert_eq!(result.body_failure, Some(BodyFailureCode::Storage));
    assert!(result.body.is_none());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM message_bodies", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    server.await.unwrap();
}

#[tokio::test]
async fn an_unwritable_failure_state_returns_storage_error_instead_of_claiming_success() {
    let mut fixture = fixture(&[None]);
    let job = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let db = rusqlite::Connection::open(fixture._directory.path().join("mailx.sqlite3")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_failure BEFORE UPDATE OF body_state ON messages
        BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;",
    )
    .unwrap();
    assert!(request.send(Err(ImapError::Timeout)).is_ok());
    assert_eq!(
        finished(job).await.unwrap_err().code,
        ErrorCode::StorageUnavailable
    );
    assert_eq!(state(&fixture, 0), BodyState::Fetching);
    assert!(
        fixture
            .manager
            .shared
            .flights
            .lock()
            .unwrap()
            .messages
            .is_empty()
    );
}

#[tokio::test]
async fn cached_body_can_rebuild_its_sanitizer_offline_without_any_connection() {
    let mut fixture = fixture(&[None]);
    let job = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (session, release, server) = session(1, 7, RAW).await;
    assert!(request.send(Ok(session)).is_ok());
    release.send(()).unwrap();
    assert_ready(&finished(job).await.unwrap());
    server.await.unwrap();
    let db = rusqlite::Connection::open(fixture._directory.path().join("mailx.sqlite3")).unwrap();
    db.execute(
        "UPDATE message_bodies SET sanitizer_version=0, plain_text='old output'",
        [],
    )
    .unwrap();
    fixture
        .manager
        .refresh(fixture.messages[0].clone())
        .await
        .unwrap();
    assert_ready(
        &fixture
            .service
            .lock()
            .unwrap()
            .get_email(&fixture.messages[0])
            .unwrap(),
    );
    assert_eq!(
        db.query_row("SELECT sanitizer_version FROM message_bodies", [], |row| {
            row.get::<_, u32>(0)
        })
        .unwrap(),
        SANITIZER_VERSION
    );
    assert!(fixture.connections.try_recv().is_err());
}

#[tokio::test]
async fn provider_policy_preserves_cached_mail_and_only_an_explicit_body_retry_reconnects() {
    let mut fixture = fixture(&[None, None]);
    let cached = fetch(&fixture, 0);
    let request = connection(&mut fixture).await;
    let (ready, release, peer) = session(1, 7, RAW).await;
    assert!(request.send(Ok(ready)).is_ok());
    release.send(()).unwrap();
    assert_ready(&finished(cached).await.unwrap());
    peer.await.unwrap();

    let blocked = fetch(&fixture, 1);
    let request = connection(&mut fixture).await;
    assert!(request.send(Err(ImapError::ProviderPolicy)).is_ok());
    let failed = finished(blocked).await.unwrap();
    assert_eq!(failed.summary.body_state, BodyState::Failed);
    assert_eq!(failed.body_failure, Some(BodyFailureCode::ProviderPolicy));
    assert!(
        !serde_json::to_string(&failed)
            .unwrap()
            .contains("Unsafe Login")
    );
    tokio::task::yield_now().await;
    assert!(fixture.connections.try_recv().is_err());
    assert_ready(
        &fixture
            .manager
            .fetch(fixture.messages[0].clone())
            .await
            .unwrap(),
    );
    assert!(fixture.connections.try_recv().is_err());

    let retry = fetch(&fixture, 1);
    let request = connection(&mut fixture).await;
    let (ready, release, peer) = session(2, 7, RAW).await;
    assert!(request.send(Ok(ready)).is_ok());
    release.send(()).unwrap();
    assert_ready(&finished(retry).await.unwrap());
    peer.await.unwrap();
}
