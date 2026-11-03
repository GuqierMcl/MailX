use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use tempfile::TempDir;

use super::*;

struct Fixture {
    _directory: TempDir,
    core: MailxCore,
    ids: Vec<String>,
    db: Connection,
    events: Arc<Mutex<Vec<MailStateChanged>>>,
}

fn fixture() -> Fixture {
    let directory = TempDir::new().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let raw = b"Content-Type: text/plain; charset=utf-8\r\n\r\nbody";
    let (core, ids) = MailxCore::body_test_fixture_with_events(
        directory.path(),
        tokio::runtime::Handle::current(),
        &[raw, raw, raw],
        Arc::new(move |event: &CoreEvent| {
            if let CoreEvent::MailStateChanged(event) = event {
                sink.lock().unwrap().push(event.clone());
            }
        }),
    )
    .unwrap();
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    Fixture {
        _directory: directory,
        core,
        ids,
        db,
        events,
    }
}

fn archived(f: &Fixture, index: usize) -> bool {
    f.core.get_email(&f.ids[index]).unwrap().summary.archived
}

#[tokio::test]
async fn flags_report_each_field_and_partial_failure_with_one_local_event() {
    let f = fixture();
    f.db.execute("UPDATE folders SET can_flag = 0", []).unwrap();
    let results = f
        .core
        .set_email_flags(
            &[
                f.ids[0].clone(),
                uuid::Uuid::new_v4().to_string(),
                f.ids[1].clone(),
            ],
            Some(true),
            Some(true),
            MailChangeSource::Mcp,
        )
        .unwrap();
    assert_eq!(results[0].is_read, Some(Ok(FlagChange::Queued)));
    assert_eq!(results[0].is_starred, Some(Ok(FlagChange::LocalOnly)));
    assert_eq!(
        results[1]
            .is_read
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        results[1]
            .is_starred
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(f.core.get_email(&f.ids[1]).unwrap().summary.is_read);
    let events = f.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, MailStateChangeKind::Flags);
    assert_eq!(events[0].source, MailChangeSource::Mcp);
    assert_eq!(events[0].changed_count, 2);
    assert_eq!(events[0].email_ids, [f.ids[0].clone(), f.ids[1].clone()]);
    assert!(events[0].undo_token.is_none());
}

#[tokio::test]
async fn flags_validate_the_batch_and_do_not_announce_noops() {
    let f = fixture();
    assert_eq!(
        f.core
            .set_email_flags(&f.ids, None, None, MailChangeSource::Ai)
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
    f.core
        .set_message_flag(&f.ids[0], super::super::MessageFlag::Read, false)
        .unwrap();
    assert!(f.events.lock().unwrap().is_empty());
    f.core
        .set_message_flag(&f.ids[0], super::super::MessageFlag::Starred, true)
        .unwrap();
    let events = f.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, MailChangeSource::User);
}

#[tokio::test]
async fn archive_refuses_the_entire_batch_without_an_event() {
    let f = fixture();
    assert_eq!(
        f.core
            .change_email_archive(
                &[f.ids[0].clone(), "missing".into()],
                true,
                MailChangeSource::Mcp
            )
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(!archived(&f, 0));
    assert!(f.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn archive_undo_restores_only_new_changes_and_is_single_use() {
    let f = fixture();
    f.core.archive_emails(&[f.ids[0].clone()]).unwrap();
    f.events.lock().unwrap().clear();
    let result = f
        .core
        .change_email_archive(&f.ids, true, MailChangeSource::Mcp)
        .unwrap();
    assert_eq!(result.processed_count, 3);
    assert_eq!(result.changed_count, 2);
    let token = result.undo_token.unwrap();
    {
        let events = f.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].undo_token.as_deref(), Some(token.as_str()));
        assert_eq!(events[0].changed_count, 2);
    }
    f.core.undo_archive(&token).unwrap();
    assert!(archived(&f, 0));
    assert!(!archived(&f, 1));
    assert!(!archived(&f, 2));
    assert_eq!(
        f.core.undo_archive(&token).unwrap_err().code,
        ErrorCode::NotFound
    );
    let events = f.events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].kind, MailStateChangeKind::UndoArchive);
    assert!(events[1].undo_token.is_none());
}

#[tokio::test]
async fn shared_message_ids_are_counted_and_restored_together() {
    let f = fixture();
    f.db.execute(
        "UPDATE messages SET message_id_header = 'shared@test' WHERE uid IN (1, 2)",
        [],
    )
    .unwrap();
    let result = f
        .core
        .change_email_archive(&[f.ids[0].clone()], true, MailChangeSource::Ai)
        .unwrap();
    assert_eq!(result.processed_count, 1);
    assert_eq!(result.changed_count, 2);
    assert!(archived(&f, 0) && archived(&f, 1));
    assert!(!archived(&f, 2));
    assert_eq!(f.events.lock().unwrap()[0].email_ids.len(), 2);
    f.core.undo_archive(&result.undo_token.unwrap()).unwrap();
    assert!(!archived(&f, 0) && !archived(&f, 1));
}

#[tokio::test]
async fn later_user_noop_invalidates_an_agent_undo() {
    let f = fixture();
    let result = f
        .core
        .change_email_archive(&[f.ids[0].clone()], true, MailChangeSource::Mcp)
        .unwrap();
    // The user expressly chooses this state after the agent. Their choice wins,
    // even though it happens to require no new database write.
    f.core.archive_emails(&[f.ids[0].clone()]).unwrap();
    assert_eq!(
        f.core
            .undo_archive(&result.undo_token.unwrap())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(archived(&f, 0));
    assert_eq!(f.events.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn unarchive_undo_does_not_overwrite_a_later_round_trip() {
    let f = fixture();
    f.core.archive_emails(&f.ids).unwrap();
    let result = f
        .core
        .change_email_archive(&f.ids, false, MailChangeSource::Mcp)
        .unwrap();
    f.core.archive_emails(&[f.ids[0].clone()]).unwrap();
    f.core.unarchive_emails(&[f.ids[0].clone()]).unwrap();
    assert_eq!(
        f.core
            .undo_archive(&result.undo_token.unwrap())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(
        f.ids
            .iter()
            .all(|id| !f.core.get_email(id).unwrap().summary.archived)
    );
}

#[tokio::test]
async fn undo_is_atomic_if_persisted_state_no_longer_matches() {
    let f = fixture();
    let result = f
        .core
        .change_email_archive(&f.ids, true, MailChangeSource::Mcp)
        .unwrap();
    f.db.execute(
        "UPDATE message_archives SET archived_at = 'changed' WHERE archive_key = ?1",
        [format!("i:{}", f.ids[1])],
    )
    .unwrap();
    assert_eq!(
        f.core
            .undo_archive(&result.undo_token.unwrap())
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(
        f.ids
            .iter()
            .all(|id| f.core.get_email(id).unwrap().summary.archived)
    );
}

#[tokio::test]
async fn expired_receipts_cannot_restore_mail() {
    let f = fixture();
    let result = f
        .core
        .change_email_archive(&f.ids, true, MailChangeSource::Mcp)
        .unwrap();
    // Expire the receipt without subtracting from the platform clock: a fresh
    // Windows runner may not have a representable instant one lifetime ago.
    f.core.archive_undo.lock().unwrap().receipts[0].expires_at = Instant::now();
    f.events.lock().unwrap().clear();
    assert_eq!(
        f.core
            .undo_archive(&result.undo_token.unwrap())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!((0..f.ids.len()).all(|index| archived(&f, index)));
    assert!(f.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn archive_event_bounds_id_hints_but_counts_every_shared_copy() {
    let f = fixture();
    f.db.execute("UPDATE messages SET message_id_header = 'shared@test'", [])
        .unwrap();
    for uid in 4..=505 {
        f.db.execute(
            "INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity,
             uid, subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
             received_at, size, is_read, is_flagged, flags, has_attachment, remote_state, updated_at, message_id_header)
             SELECT ?1, account_id, mailbox_source_id, folder_id, uid_validity,
             ?2, subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
             received_at, size, is_read, is_flagged, flags, has_attachment, remote_state, updated_at, message_id_header
             FROM messages WHERE id = ?3",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), uid, f.ids[0]],
        ).unwrap();
    }
    let result = f
        .core
        .change_email_archive(&[f.ids[0].clone()], true, MailChangeSource::Mcp)
        .unwrap();
    assert_eq!(result.changed_count, 505);
    let events = f.events.lock().unwrap();
    assert_eq!(events[0].changed_count, 505);
    assert_eq!(events[0].email_ids.len(), 500);
    assert_eq!(events[0].account_ids.len(), 1);
}
