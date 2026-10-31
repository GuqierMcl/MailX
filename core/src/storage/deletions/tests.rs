use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use crate::{
    accounts::{AccountService, sync_plan::tests::config},
    error::ErrorCode,
    imap::FolderStatus,
    mail::{EmailListQuery, FolderRole},
    secrets::test_support::MemorySecrets,
    storage::messages::SyncTarget,
    sync::{folders::FolderRecord, headers::MessageRecord},
};

struct Fixture {
    service: AccountService,
    db: Connection,
    account_id: String,
    inbox: SyncTarget,
    trash: SyncTarget,
    _directory: TempDir,
}

fn folder(name: &str, role: FolderRole, unseen: u32) -> FolderRecord {
    FolderRecord {
        remote_name: name.into(),
        parent_remote_name: None,
        name: name.into(),
        delimiter: Some("/".into()),
        role,
        selectable: true,
        attributes: vec![],
        status: Some(FolderStatus {
            messages: 10,
            unseen,
            uid_next: Some(100),
            uid_validity: Some(10),
        }),
    }
}

fn record(uid: u32, is_read: bool) -> MessageRecord {
    MessageRecord {
        uid,
        message_id_header: Some(format!("message-{uid}@example.test")),
        subject: format!("Subject {uid}"),
        from_name: None,
        from_address: "sender@example.test".into(),
        to_addresses: vec![],
        cc_addresses: vec![],
        reply_to_addresses: vec![],
        received_at: "2026-10-18T09:00:00.000Z".into(),
        sent_at: None,
        size: Some(400),
        is_read,
        is_flagged: false,
        flags: vec![],
        has_attachment: false,
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap()
}

fn setup(with_trash: bool) -> Fixture {
    let directory = TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let mut service = AccountService::open(directory.path(), secrets).unwrap();
    let account = service
        .create(
            serde_json::from_value(json!({
                "requestId": Uuid::new_v4().to_string(),
                "config": config(json!({ "kind": "direct" })),
            }))
            .unwrap(),
        )
        .unwrap();
    let mut folders = vec![folder("INBOX", FolderRole::Inbox, 2)];
    if with_trash {
        folders.push(folder("Trash", FolderRole::Trash, 0));
    }
    service
        .store_folders(&account.id, &account.mailbox_source_id, &folders)
        .unwrap();
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let folder_id = |name: &str| -> String {
        db.query_row(
            "SELECT id FROM folders WHERE remote_name = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap_or_default()
    };
    let target = |folder_id: String| SyncTarget {
        account_id: account.id.clone(),
        mailbox_source_id: account.mailbox_source_id.clone(),
        folder_id,
        uid_validity: 10,
    };
    let inbox = target(folder_id("INBOX"));
    let trash = target(folder_id("Trash"));
    service
        .ensure_generation(&inbox, "2026-09-18")
        .unwrap()
        .unwrap();
    service
        .store_message_batch(
            &inbox,
            &[record(1, false), record(2, true), record(3, false)],
            3,
        )
        .unwrap()
        .unwrap();
    if with_trash {
        service
            .ensure_generation(&trash, "2026-09-18")
            .unwrap()
            .unwrap();
        service
            .store_message_batch(&trash, &[record(8, true)], 8)
            .unwrap()
            .unwrap();
    }
    // Header syncs mark the folder synced; the unread number is the server's.
    db.execute(
        "UPDATE folders SET unread_count = 2, synced_at = '2026-10-31T00:00:00.000Z'",
        [],
    )
    .unwrap();
    Fixture {
        service,
        db,
        account_id: account.id,
        inbox,
        trash,
        _directory: directory,
    }
}

fn id(f: &Fixture, folder: &SyncTarget, uid: u32) -> String {
    f.db.query_row(
        "SELECT id FROM messages WHERE folder_id = ?1 AND uid = ?2",
        rusqlite::params![folder.folder_id, uid],
        |row| row.get(0),
    )
    .unwrap()
}

fn visible(f: &Fixture, role: FolderRole) -> Vec<String> {
    f.service
        .list_emails(&EmailListQuery {
            folder_role: Some(role),
            ..Default::default()
        })
        .unwrap()
        .items
        .into_iter()
        .map(|email| email.subject)
        .collect()
}

fn unread(f: &Fixture) -> i64 {
    f.db.query_row(
        "SELECT unread_count FROM folders WHERE id = ?1",
        [&f.inbox.folder_id],
        |row| row.get(0),
    )
    .unwrap()
}

fn remote_state(f: &Fixture, message_id: &str) -> String {
    f.db.query_row(
        "SELECT remote_state FROM messages WHERE id = ?1",
        [message_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn deleting_hides_the_message_at_once_and_schedules_a_move_to_trash() {
    let mut f = setup(true);
    let first = id(&f, &f.inbox.clone(), 1);
    let scheduled = f
        .service
        .schedule_deletion(
            std::slice::from_ref(&first),
            false,
            now(),
            Duration::seconds(5),
        )
        .unwrap();
    assert!(!scheduled.permanent);
    assert_eq!(scheduled.due_at, now() + Duration::seconds(5));
    assert_eq!(scheduled.account_ids, [f.account_id.clone()]);
    assert!(!visible(&f, FolderRole::Inbox).contains(&"Subject 1".to_owned()));
    // An unread message stops counting as unread while it is hidden.
    assert_eq!(unread(&f), 1);
    assert!(matches!(
        f.service.get_email(&first).map_err(|error| error.code),
        Err(ErrorCode::NotFound)
    ));
    // Asking again for a hidden message reads as missing.
    let again = f
        .service
        .schedule_deletion(&[first], false, now(), Duration::seconds(5))
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::NotFound);
}

#[test]
fn deleting_for_good_needs_confirmation_in_trash_and_without_trash() {
    let mut f = setup(true);
    let in_trash = id(&f, &f.trash.clone(), 8);
    let error = f
        .service
        .schedule_deletion(
            std::slice::from_ref(&in_trash),
            false,
            now(),
            Duration::seconds(5),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ConfirmationRequired);
    assert!(visible(&f, FolderRole::Trash).contains(&"Subject 8".to_owned()));
    let scheduled = f
        .service
        .schedule_deletion(&[in_trash], true, now(), Duration::seconds(5))
        .unwrap();
    assert!(scheduled.permanent);

    let mut f = setup(false);
    let message = id(&f, &f.inbox.clone(), 2);
    let error = f
        .service
        .schedule_deletion(
            std::slice::from_ref(&message),
            false,
            now(),
            Duration::seconds(5),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ConfirmationRequired);
    assert!(
        f.service
            .schedule_deletion(&[message], true, now(), Duration::seconds(5))
            .unwrap()
            .permanent
    );
}

#[test]
fn a_refused_batch_hides_nothing() {
    let mut f = setup(true);
    let ids = [id(&f, &f.inbox.clone(), 1), id(&f, &f.trash.clone(), 8)];
    let error = f
        .service
        .schedule_deletion(&ids, false, now(), Duration::seconds(5))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ConfirmationRequired);
    assert_eq!(visible(&f, FolderRole::Inbox).len(), 3);
    assert_eq!(unread(&f), 2);
    let empty = f
        .service
        .schedule_deletion(&[], true, now(), Duration::seconds(5))
        .unwrap_err();
    assert_eq!(empty.code, ErrorCode::Validation);
}

#[test]
fn undo_before_the_worker_starts_shows_the_message_again() {
    let mut f = setup(true);
    let first = id(&f, &f.inbox.clone(), 1);
    let scheduled = f
        .service
        .schedule_deletion(&[first], false, now(), Duration::seconds(5))
        .unwrap();
    assert_eq!(
        f.service.next_deletion_due(&f.account_id).unwrap(),
        Some(scheduled.due_at)
    );
    // Not due yet: the worker claims nothing and undo still works.
    let (due, given_back) = f
        .service
        .claim_due_deletions(&f.account_id, now() + Duration::seconds(4))
        .unwrap();
    assert!(due.is_empty());
    assert_eq!(given_back, 0);
    assert_eq!(
        f.service.cancel_deletion(&scheduled.batch_id).unwrap(),
        [f.account_id.clone()]
    );
    assert_eq!(visible(&f, FolderRole::Inbox).len(), 3);
    assert_eq!(unread(&f), 2);
    assert_eq!(f.service.next_deletion_due(&f.account_id).unwrap(), None);
    let gone = f.service.cancel_deletion(&scheduled.batch_id).unwrap_err();
    assert_eq!(gone.code, ErrorCode::NotFound);
}

#[test]
fn once_claimed_undo_is_refused_and_success_leaves_a_tombstone() {
    let mut f = setup(true);
    let first = id(&f, &f.inbox.clone(), 1);
    let scheduled = f
        .service
        .schedule_deletion(
            std::slice::from_ref(&first),
            false,
            now(),
            Duration::seconds(5),
        )
        .unwrap();
    let (due, _) = f
        .service
        .claim_due_deletions(&f.account_id, now() + Duration::seconds(5))
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].message_id, first);
    assert_eq!(due[0].folder_remote_name, "INBOX");
    assert_eq!(due[0].target_remote_name.as_deref(), Some("Trash"));
    assert_eq!((due[0].uid_validity, due[0].uid), (10, 1));
    let refused = f.service.cancel_deletion(&scheduled.batch_id).unwrap_err();
    assert_eq!(refused.code, ErrorCode::AlreadyDeleting);
    // A claimed row is not "next due"; a failed round claims it again.
    assert_eq!(f.service.next_deletion_due(&f.account_id).unwrap(), None);
    let (again, _) = f.service.claim_due_deletions(&f.account_id, now()).unwrap();
    assert_eq!(again.len(), 1);

    f.service
        .finish_deletions(std::slice::from_ref(&first))
        .unwrap();
    assert_eq!(remote_state(&f, &first), "deleted");
    assert!(!visible(&f, FolderRole::Inbox).contains(&"Subject 1".to_owned()));
    let rows: u32 =
        f.db.query_row("SELECT COUNT(*) FROM message_deletions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
    // The unread number stays down until the server's own count arrives.
    assert_eq!(unread(&f), 1);
}

#[test]
fn deletions_that_keep_failing_or_lost_their_generation_come_back() {
    let mut f = setup(true);
    let first = id(&f, &f.inbox.clone(), 1);
    let third = id(&f, &f.inbox.clone(), 3);
    f.service
        .schedule_deletion(
            &[first.clone(), third.clone()],
            false,
            now(),
            Duration::zero(),
        )
        .unwrap();
    f.service.claim_due_deletions(&f.account_id, now()).unwrap();
    for _ in 1..super::MAX_DELETION_ATTEMPTS {
        assert_eq!(
            f.service
                .deletion_attempt_failed(std::slice::from_ref(&first))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        f.service
            .deletion_attempt_failed(std::slice::from_ref(&first))
            .unwrap(),
        1
    );
    assert!(visible(&f, FolderRole::Inbox).contains(&"Subject 1".to_owned()));
    assert_eq!(remote_state(&f, &first), "present");

    // The folder got a new UIDVALIDITY: the stored UID means nothing now.
    f.db.execute(
        "UPDATE folder_sync_cursors SET uid_validity = 11 WHERE folder_id = ?1",
        [&f.inbox.folder_id],
    )
    .unwrap();
    let (due, given_back) = f.service.claim_due_deletions(&f.account_id, now()).unwrap();
    assert!(due.is_empty());
    assert_eq!(given_back, 1);
    assert_eq!(unread(&f), 2);
    assert_eq!(
        f.db.query_row("SELECT COUNT(*) FROM message_deletions", [], |row| row
            .get::<_, u32>(0))
            .unwrap(),
        0
    );
}

#[test]
fn a_trash_folder_that_disappeared_gives_the_deletion_back() {
    let mut f = setup(true);
    let first = id(&f, &f.inbox.clone(), 1);
    f.service
        .schedule_deletion(std::slice::from_ref(&first), false, now(), Duration::zero())
        .unwrap();
    f.db.execute("UPDATE folders SET selectable = 0 WHERE role = 'trash'", [])
        .unwrap();
    assert_eq!(unread(&f), 1);
    let (due, given_back) = f.service.claim_due_deletions(&f.account_id, now()).unwrap();
    assert!(due.is_empty());
    assert_eq!(given_back, 1);
    assert_eq!(unread(&f), 2);
    assert!(visible(&f, FolderRole::Inbox).contains(&"Subject 1".to_owned()));
    // Giving back what is already back changes nothing, unread number included.
    f.service.abandon_deletions(&[first]).unwrap();
    assert_eq!(unread(&f), 2);
}

#[test]
fn the_server_count_does_not_bring_back_a_hidden_unread_message() {
    let mut f = setup(true);
    let first = id(&f, &f.inbox.clone(), 1);
    let scheduled = f
        .service
        .schedule_deletion(&[first], false, now(), Duration::seconds(5))
        .unwrap();
    let inbox = f.inbox.clone();
    // A header round inside the undo window still sees the message unread.
    f.service
        .update_folder_counts(
            &inbox.account_id,
            &inbox.mailbox_source_id,
            &inbox.folder_id,
            2,
            10,
        )
        .unwrap();
    assert_eq!(unread(&f), 1);
    f.service.cancel_deletion(&scheduled.batch_id).unwrap();
    f.service
        .update_folder_counts(
            &inbox.account_id,
            &inbox.mailbox_source_id,
            &inbox.folder_id,
            2,
            10,
        )
        .unwrap();
    assert_eq!(unread(&f), 2);
}
