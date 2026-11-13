use std::sync::Arc;

use rusqlite::Connection;
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::{
    mail::{BodyState, render},
    secrets::test_support::MemorySecrets,
    sync::bodies::tests::{create_account, seed, session},
};

const RAW: &[u8] = b"Subject: Your verification code\r\nContent-Type: text/plain\r\n\r\nYour verification code: 482913";

struct Fixture {
    _directory: TempDir,
    core: MailxCore,
    db: Connection,
    id: String,
    account: String,
    connections: mpsc::UnboundedReceiver<oneshot::Sender<Result<imap::Session, imap::ImapError>>>,
}

fn fixture(cached: bool) -> Fixture {
    let directory = TempDir::new().unwrap();
    let core = MailxCore::open(
        directory.path(),
        Arc::new(MemorySecrets::default()),
        tokio::runtime::Handle::current(),
        Arc::new(|_: &crate::CoreEvent| {}),
    )
    .unwrap();
    let (account, id) = {
        let mut service = core.accounts().unwrap();
        let account = create_account(&mut service, "notification");
        let (_, ids) = seed(&mut service, &account, &[Some(RAW.len() as u32)]);
        (account.id, ids[0].clone())
    };
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    db.execute(
        "UPDATE messages SET subject = 'Your verification code', received_at = ?1",
        [Utc::now().to_rfc3339()],
    )
    .unwrap();
    if cached {
        let (identity, blobs) = {
            let service = core.accounts().unwrap();
            (service.body_identity(&id).unwrap(), service.body_blobs())
        };
        let blob = blobs.put(RAW).unwrap();
        let rendered = render::render(RAW, &id).unwrap();
        let mut service = core.accounts().unwrap();
        assert!(service.body_begin(&identity).unwrap());
        assert!(service.body_commit(&identity, &blob, &rendered).unwrap());
    }
    let (sender, connections) = mpsc::unbounded_channel();
    *core.notification_codes.connector.lock().unwrap() = Some(sender);
    Fixture {
        _directory: directory,
        core,
        db,
        id,
        account,
        connections,
    }
}

#[tokio::test]
async fn cached_notification_detection_has_no_network_or_state_effects() {
    let mut f = fixture(true);
    let before = f.core.get_email(&f.id).unwrap();
    let code = f
        .core
        .notification_verification_code(f.id.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(code.code, "482913");
    assert_eq!(f.core.get_email(&f.id).unwrap(), before);
    assert!(f.connections.try_recv().is_err());
}

#[tokio::test]
async fn only_current_unread_recent_inbox_candidates_are_fetched() {
    for update in [
        "UPDATE messages SET is_read = 1",
        "UPDATE messages SET received_at = '2000-01-01T00:00:00Z'",
        "UPDATE messages SET received_at = '2200-01-01T00:00:00Z'",
        "UPDATE messages SET subject = 'Monthly invoice'",
        "UPDATE messages SET remote_state = 'deleted'",
        "UPDATE folders SET role = 'sent'",
        "UPDATE accounts SET mailbox_source_id = 'different-source'",
        "UPDATE folder_sync_cursors SET uid_validity = 8",
        "UPDATE messages SET size = 1048577",
    ] {
        let mut f = fixture(false);
        f.db.execute(update, []).unwrap();
        assert!(
            f.core
                .notification_verification_code(f.id.clone())
                .await
                .unwrap()
                .is_none(),
            "{update}"
        );
        assert!(f.connections.try_recv().is_err(), "{update}");
    }
}

#[tokio::test]
async fn click_revalidation_binds_account_and_allows_read_but_not_old_or_deleted_mail() {
    let f = fixture(true);
    assert!(
        f.core
            .notification_code_is_current(&f.account, &f.id)
            .unwrap()
    );
    assert!(
        !f.core
            .notification_code_is_current("different-account", &f.id)
            .unwrap()
    );
    f.db.execute("UPDATE messages SET is_read = 1", []).unwrap();
    assert!(
        f.core
            .notification_code_is_current(&f.account, &f.id)
            .unwrap()
    );
    f.db.execute("UPDATE messages SET remote_state = 'deleted'", [])
        .unwrap();
    assert!(
        !f.core
            .notification_code_is_current(&f.account, &f.id)
            .unwrap()
    );
}

#[tokio::test]
async fn network_uses_read_only_peek_and_keeps_body_missing() {
    let mut f = fixture(false);
    let core = f.core.clone();
    let id = f.id.clone();
    let task = tokio::spawn(async move { core.notification_verification_code(id).await });
    let requested = f.connections.recv().await.unwrap();
    let (connection, release, server) = session(1, 7, RAW).await;
    requested.send(Ok(connection)).ok().unwrap();
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap().unwrap().code, "482913");
    server.await.unwrap();
    let detail = f.core.get_email(&f.id).unwrap();
    assert_eq!(detail.summary.body_state, BodyState::Missing);
    assert!(!detail.summary.is_read);
    let rows: i64 =
        f.db.query_row("SELECT COUNT(*) FROM message_bodies", [], |row| row.get(0))
            .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn delayed_network_result_cannot_survive_account_revision_change() {
    let mut f = fixture(false);
    let core = f.core.clone();
    let id = f.id.clone();
    let task = tokio::spawn(async move { core.notification_verification_code(id).await });
    let requested = f.connections.recv().await.unwrap();
    let (connection, release, server) = session(1, 7, RAW).await;
    requested.send(Ok(connection)).ok().unwrap();
    f.db.execute("UPDATE accounts SET revision = revision + 1", [])
        .unwrap();
    release.send(()).unwrap();
    assert!(task.await.unwrap().unwrap().is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn uidvalidity_mismatch_falls_back_without_fetching() {
    let mut f = fixture(false);
    let core = f.core.clone();
    let id = f.id.clone();
    let task = tokio::spawn(async move { core.notification_verification_code(id).await });
    let requested = f.connections.recv().await.unwrap();
    let (connection, _release, server) = session(1, 8, RAW).await;
    requested.send(Ok(connection)).ok().unwrap();
    assert!(task.await.unwrap().unwrap().is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn cancelling_a_caller_drops_its_pending_connection_and_releases_the_gate() {
    let mut f = fixture(false);
    let core = f.core.clone();
    let id = f.id.clone();
    let task = tokio::spawn(async move { core.notification_verification_code(id).await });
    let requested = f.connections.recv().await.unwrap();
    assert_eq!(f.core.notification_codes.gate.available_permits(), 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(requested.is_closed());
    assert_eq!(f.core.notification_codes.gate.available_permits(), 2);
    assert_eq!(
        f.core.get_email(&f.id).unwrap().summary.body_state,
        BodyState::Missing
    );
}

#[tokio::test]
async fn global_gate_limits_parallel_requests_and_timeout_discards_waiting_work() {
    let mut f = fixture(false);
    let mut jobs = Vec::new();
    for _ in 0..3 {
        let core = f.core.clone();
        let id = f.id.clone();
        jobs.push(tokio::spawn(async move {
            core.notification_verification_code(id).await
        }));
    }
    let first = f.connections.recv().await.unwrap();
    let second = f.connections.recv().await.unwrap();
    assert!(f.connections.try_recv().is_err());
    for task in jobs {
        assert!(task.await.unwrap().unwrap().is_none());
    }
    assert!(first.is_closed() && second.is_closed());
    assert_eq!(f.core.notification_codes.gate.available_permits(), 2);
}
