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
    let ids = {
        let mut service = f.core.accounts().unwrap();
        let account = service.get(&f.account).unwrap();
        seed(&mut service, &account, &[Some(RAW.len() as u32); 3]).1
    };
    f.db.execute(
        "UPDATE messages SET subject = 'Your verification code', received_at = ?1",
        [Utc::now().to_rfc3339()],
    )
    .unwrap();
    let mut jobs = Vec::new();
    for id in ids {
        let core = f.core.clone();
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

fn cache_raw(f: &Fixture, raw: &[u8]) {
    let (identity, blobs) = {
        let service = f.core.accounts().unwrap();
        (service.body_identity(&f.id).unwrap(), service.body_blobs())
    };
    let blob = blobs.put(raw).unwrap();
    let rendered = render::render(raw, &f.id).unwrap();
    let mut service = f.core.accounts().unwrap();
    if !service.body_begin(&identity).unwrap() {
        assert!(service.body_rebuild(&identity).unwrap());
    }
    assert!(service.body_commit(&identity, &blob, &rendered).unwrap());
}

fn add_trash(f: &Fixture) {
    use crate::{mail::FolderRole, sync::folders::FolderRecord};
    let mut service = f.core.accounts().unwrap();
    let account = service.get(&f.account).unwrap();
    let folders =
        [("INBOX", FolderRole::Inbox), ("Trash", FolderRole::Trash)].map(|(name, role)| {
            FolderRecord {
                remote_name: name.into(),
                parent_remote_name: None,
                name: name.into(),
                delimiter: Some("/".into()),
                role,
                selectable: true,
                attributes: vec![],
                status: None,
            }
        });
    service
        .store_folders(&account.id, &account.mailbox_source_id, &folders)
        .unwrap();
}

async fn recognize(f: &Fixture) -> VerificationCodeStatus {
    let status = f
        .core
        .verification_code_status(f.id.clone(), false)
        .await
        .unwrap();
    assert_eq!(status.state, VerificationCodeState::Ready);
    assert_eq!(status.code.as_deref(), Some("482913"));
    assert!(
        status
            .recognition_id
            .as_deref()
            .is_some_and(|id| Uuid::parse_str(id).is_ok())
    );
    status
}

#[tokio::test]
async fn ui_status_and_copy_work_for_read_old_archived_mail_with_any_cached_subject() {
    let mut f = fixture(true);
    f.db.execute("UPDATE messages SET is_read = 1, received_at = '2000-01-01T00:00:00Z', subject = 'Account details'", []).unwrap();
    f.db.execute("UPDATE folders SET role = 'archive'", [])
        .unwrap();
    cache_raw(
        &f,
        b"Subject: Account details\r\nContent-Type: text/plain\r\n\r\nVerification code: 482913",
    );
    assert!(
        f.core
            .notification_verification_code(f.id.clone())
            .await
            .unwrap()
            .is_none()
    );
    let status = recognize(&f).await;
    let mut copied = String::new();
    let result = f
        .core
        .copy_verification_code(
            &f.id,
            status.recognition_id.as_deref().unwrap(),
            false,
            |code| {
                copied = code.to_owned();
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(copied, "482913");
    assert_eq!(result.cleanup, VerificationCodeCleanup::Disabled);
    assert!(result.deletion.is_none());
    assert!(f.core.get_email(&f.id).unwrap().summary.is_read);
    assert!(f.connections.try_recv().is_err());
}

#[tokio::test]
async fn ui_does_not_download_uncached_ordinary_mail_or_without_permission() {
    let mut f = fixture(false);
    let status = f
        .core
        .verification_code_status(f.id.clone(), false)
        .await
        .unwrap();
    assert_eq!(status.state, VerificationCodeState::Unchecked);
    f.db.execute("UPDATE messages SET subject = 'Weekly report'", [])
        .unwrap();
    let status = f
        .core
        .verification_code_status(f.id.clone(), true)
        .await
        .unwrap();
    assert_eq!(status.state, VerificationCodeState::Unchecked);
    assert!(f.connections.try_recv().is_err());
    cache_raw(&f, RAW);
    recognize(&f).await;
}

#[tokio::test]
async fn recognized_mail_is_available_even_when_both_network_slots_are_busy() {
    let f = fixture(true);
    let expected = recognize(&f).await;
    let _busy = f
        .core
        .notification_codes
        .gate
        .acquire_many(2)
        .await
        .unwrap();
    let status = tokio::time::timeout(
        Duration::from_secs(1),
        f.core.verification_code_status(f.id.clone(), false),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(status, expected);
}

#[tokio::test]
async fn notification_and_ui_share_a_result_and_concurrent_reads_share_one_download() {
    let mut f = fixture(false);
    let mut jobs = Vec::new();
    for _ in 0..3 {
        let core = f.core.clone();
        let id = f.id.clone();
        jobs.push(tokio::spawn(async move {
            core.verification_code_status(id, true).await.unwrap()
        }));
    }
    let requested = f.connections.recv().await.unwrap();
    tokio::task::yield_now().await;
    assert!(f.connections.try_recv().is_err());
    let (connection, release, server) = session(1, 7, RAW).await;
    requested.send(Ok(connection)).ok().unwrap();
    release.send(()).unwrap();
    let mut statuses = Vec::new();
    for job in jobs {
        statuses.push(job.await.unwrap());
    }
    assert!(statuses.iter().all(|status| status == &statuses[0]));
    assert_eq!(statuses[0].state, VerificationCodeState::Ready);
    assert_eq!(
        f.core
            .notification_verification_code(f.id.clone())
            .await
            .unwrap()
            .unwrap()
            .code,
        "482913"
    );
    assert_eq!(
        f.core.cached_verification_code_status(&f.id).unwrap(),
        statuses[0]
    );
    assert!(f.connections.try_recv().is_err());
    server.await.unwrap();
    // A later normal reader download of exactly the same MIME keeps the token.
    cache_raw(&f, RAW);
    assert_eq!(
        f.core.cached_verification_code_status(&f.id).unwrap(),
        statuses[0]
    );
}

#[tokio::test]
async fn body_version_changes_invalidate_old_capabilities_and_recognize_again() {
    let f = fixture(true);
    let status = recognize(&f).await;
    cache_raw(
        &f,
        b"Subject: Account details\r\nContent-Type: text/plain\r\n\r\nVerification code: 762951",
    );
    let result = f.core.copy_verification_code(
        &f.id,
        status.recognition_id.as_deref().unwrap(),
        true,
        |_| panic!("stale code must not copy"),
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        f.core.cached_verification_code_status(&f.id).unwrap().state,
        VerificationCodeState::Unchecked
    );
    let next = f
        .core
        .verification_code_status(f.id.clone(), false)
        .await
        .unwrap();
    assert_eq!(next.code.as_deref(), Some("762951"));
    assert_ne!(next.recognition_id, status.recognition_id);
}

#[tokio::test]
async fn stale_identity_unknown_capability_and_deleted_mail_never_invoke_clipboard() {
    for update in [
        "UPDATE accounts SET revision = revision + 1",
        "UPDATE accounts SET mailbox_source_id = 'changed-source'",
        "UPDATE folder_sync_cursors SET uid_validity = 8",
        "UPDATE messages SET remote_state = 'deleted'",
    ] {
        let f = fixture(true);
        let status = recognize(&f).await;
        f.db.execute(update, []).unwrap();
        assert!(
            f.core
                .copy_verification_code(
                    &f.id,
                    status.recognition_id.as_deref().unwrap(),
                    false,
                    |_| panic!("stale code must not copy")
                )
                .is_err(),
            "{update}"
        );
    }
    let f = fixture(true);
    recognize(&f).await;
    assert_eq!(
        f.core
            .copy_verification_code(&f.id, &Uuid::new_v4().to_string(), false, |_| panic!(
                "unknown capability"
            ))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn clipboard_failure_never_queues_cleanup_and_a_later_retry_can_succeed() {
    let f = fixture(true);
    add_trash(&f);
    let status = recognize(&f).await;
    let error = f
        .core
        .copy_verification_code(
            &f.id,
            status.recognition_id.as_deref().unwrap(),
            true,
            |_| Err(internal()),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Internal);
    assert!(f.core.get_email(&f.id).is_ok());
    let count: u32 =
        f.db.query_row("SELECT COUNT(*) FROM message_deletions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let result = f
        .core
        .copy_verification_code(
            &f.id,
            status.recognition_id.as_deref().unwrap(),
            true,
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(result.cleanup, VerificationCodeCleanup::Queued);
}

#[tokio::test]
async fn copy_policy_is_sampled_after_waiting_for_service_admission() {
    use std::sync::atomic::{AtomicBool, Ordering};

    for latest in [false, true] {
        let f = fixture(true);
        add_trash(&f);
        let status = recognize(&f).await;
        let preference = Arc::new(AtomicBool::new(!latest));
        let copied = Arc::new(AtomicBool::new(false));
        let held = f.core.accounts().unwrap();
        let (started, queued) = std::sync::mpsc::channel();
        let core = f.core.clone();
        let id = f.id.clone();
        let policy = preference.clone();
        let writes = copied.clone();
        let operation = std::thread::spawn(move || {
            started.send(()).unwrap();
            core.copy_verification_code_with_policy(
                &id,
                status.recognition_id.as_deref().unwrap(),
                |code| {
                    assert_eq!(code, "482913");
                    writes.store(true, Ordering::SeqCst);
                    Ok(policy.load(Ordering::SeqCst))
                },
            )
        });
        queued.recv().unwrap();
        assert!(!copied.load(Ordering::SeqCst));
        preference.store(latest, Ordering::SeqCst);
        drop(held);
        let result = operation.join().unwrap().unwrap();
        assert!(copied.load(Ordering::SeqCst));
        assert_eq!(
            result.cleanup,
            if latest {
                VerificationCodeCleanup::Queued
            } else {
                VerificationCodeCleanup::Disabled
            }
        );
    }
}

#[tokio::test]
async fn successful_copy_moves_only_one_mail_to_trash_with_undo_and_no_duplicate_deletion() {
    let f = fixture(true);
    add_trash(&f);
    let status = recognize(&f).await;
    let before = Utc::now();
    let mut count = 0;
    let result = f
        .core
        .copy_verification_code(
            &f.id,
            status.recognition_id.as_deref().unwrap(),
            true,
            |code| {
                assert_eq!(code, "482913");
                count += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(result.cleanup, VerificationCodeCleanup::Queued);
    assert_eq!(result.email_id, f.id);
    assert_eq!(result.account_id, f.account);
    let deletion = result.deletion.unwrap();
    assert!(!deletion.permanent);
    assert!(
        DateTime::parse_from_rfc3339(&deletion.undo_until).unwrap()
            >= before + chrono::Duration::seconds(4)
    );
    assert!(f.core.get_email(&f.id).is_err());
    assert!(
        f.core
            .copy_verification_code(
                &f.id,
                status.recognition_id.as_deref().unwrap(),
                true,
                |_| {
                    count += 1;
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(count, 1);
    let queued: u32 =
        f.db.query_row("SELECT COUNT(*) FROM message_deletions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(queued, 1);
    f.core.undo_delete(&deletion.batch_id).unwrap();
    assert!(f.core.get_email(&f.id).is_ok());
}

#[tokio::test]
async fn no_trash_or_already_trashed_mail_only_copies_and_never_expunges() {
    for in_trash in [false, true] {
        let f = fixture(true);
        if in_trash {
            f.db.execute("UPDATE folders SET role = 'trash'", [])
                .unwrap();
        }
        let status = recognize(&f).await;
        let mut copied = false;
        let result = f
            .core
            .copy_verification_code(
                &f.id,
                status.recognition_id.as_deref().unwrap(),
                true,
                |_| {
                    copied = true;
                    Ok(())
                },
            )
            .unwrap();
        assert!(copied);
        assert_eq!(result.cleanup, VerificationCodeCleanup::Unavailable);
        assert!(result.deletion.is_none());
        assert!(f.core.get_email(&f.id).is_ok());
        let queued: u32 =
            f.db.query_row("SELECT COUNT(*) FROM message_deletions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(queued, 0);
    }
}

#[tokio::test]
async fn scheduling_failure_is_reported_after_successful_copy_without_losing_mail() {
    let f = fixture(true);
    add_trash(&f);
    let status = recognize(&f).await;
    f.db.execute_batch("CREATE TRIGGER reject_cleanup BEFORE INSERT ON message_deletions BEGIN SELECT RAISE(FAIL, 'synthetic'); END;").unwrap();
    let mut copied = false;
    let result = f
        .core
        .copy_verification_code(
            &f.id,
            status.recognition_id.as_deref().unwrap(),
            true,
            |_| {
                copied = true;
                Ok(())
            },
        )
        .unwrap();
    assert!(copied);
    assert_eq!(result.cleanup, VerificationCodeCleanup::Failed);
    assert!(result.deletion.is_none());
    assert!(f.core.get_email(&f.id).is_ok());
}

#[tokio::test]
async fn negative_and_ambiguous_results_are_shared_without_exposing_candidates() {
    let f = fixture(true);
    for (raw, expected) in [
        (b"Subject: Report\r\nContent-Type: text/plain\r\n\r\nNothing to verify".as_slice(), VerificationCodeState::NotFound),
        (b"Subject: Verification code\r\nContent-Type: text/plain\r\n\r\nVerification code: 482913 or 762951".as_slice(), VerificationCodeState::Ambiguous),
    ] {
        cache_raw(&f, raw);
        let status = f.core.verification_code_status(f.id.clone(), false).await.unwrap();
        assert_eq!(status.state, expected);
        assert!(status.code.is_none() && status.recognition_id.is_none());
        assert_eq!(f.core.cached_verification_code_status(&f.id).unwrap(), status);
    }
    cache_raw(&f, RAW);
    let ready = recognize(&f).await;
    assert!(!format!("{ready:?}").contains("482913"));
    assert!(!format!("{ready:?}").contains(ready.recognition_id.as_deref().unwrap()));
    assert!(serde_json::to_string(&ready).unwrap().contains("482913"));
}
