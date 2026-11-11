use std::sync::Arc;

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use crate::{
    accounts::{AccountService, sync_plan::tests::config},
    error::ErrorCode,
    imap::FolderStatus,
    mail::{ArchiveScope, EmailListQuery, FolderRole},
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
        original_message_id: None,
        sent_identity_policy: crate::providers::for_imap_host("").policy_key(),
        in_reply_to: None,
        references_header: None,
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

fn list(f: &Fixture, role: FolderRole, archive: Option<ArchiveScope>) -> Vec<String> {
    f.service
        .list_emails(&EmailListQuery {
            folder_role: Some(role),
            archive,
            ..Default::default()
        })
        .unwrap()
        .items
        .into_iter()
        .map(|email| email.subject)
        .collect()
}

fn inbox_folder(f: &Fixture) -> crate::mail::Folder {
    f.service
        .list_folders(&f.account_id)
        .unwrap()
        .into_iter()
        .find(|folder| folder.role == FolderRole::Inbox)
        .unwrap()
}

fn code(result: Result<(), crate::error::AppError>) -> ErrorCode {
    result.unwrap_err().code
}

#[test]
fn archived_mail_leaves_the_inbox_and_fills_the_archived_view() {
    let mut f = setup(true);
    let one = id(&f, &f.inbox, 1);
    f.service
        .archive_messages(std::slice::from_ref(&one))
        .unwrap();
    let mut inbox = list(&f, FolderRole::Inbox, None);
    inbox.sort();
    assert_eq!(inbox, ["Subject 2", "Subject 3"]);
    assert_eq!(
        list(&f, FolderRole::Inbox, Some(ArchiveScope::Only)),
        ["Subject 1"]
    );
    assert_eq!(
        list(&f, FolderRole::Inbox, Some(ArchiveScope::Include)).len(),
        3
    );
    // Archiving again is harmless; opening the message still works.
    f.service
        .archive_messages(std::slice::from_ref(&one))
        .unwrap();
    assert!(f.service.get_email(&one).unwrap().summary.archived);
    // Per-folder views follow the same rule.
    let by_folder = f
        .service
        .list_emails(&EmailListQuery {
            folder_id: Some(f.inbox.folder_id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(by_folder.items.len(), 2);
}

#[test]
fn archived_views_are_scoped_to_the_requested_accounts() {
    let mut f = setup(false);
    f.service.archive_messages(&[id(&f, &f.inbox, 2)]).unwrap();
    let only = |accounts: Vec<String>| {
        f.service
            .list_emails(&EmailListQuery {
                account_ids: Some(accounts),
                folder_role: Some(FolderRole::Inbox),
                archive: Some(ArchiveScope::Only),
                ..Default::default()
            })
            .unwrap()
            .items
            .len()
    };
    assert_eq!(only(vec![f.account_id.clone()]), 1);
    assert_eq!(only(vec![Uuid::new_v4().to_string()]), 0);
}

#[test]
fn only_visible_inbox_messages_can_be_archived_and_batches_are_all_or_nothing() {
    let mut f = setup(true);
    let one = id(&f, &f.inbox, 1);
    let trashed = id(&f, &f.trash, 8);
    assert_eq!(
        code(f.service.archive_messages(&[one.clone(), trashed])),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(f.service.archive_messages(&[one.clone(), "missing".into()])),
        ErrorCode::NotFound
    );
    assert_eq!(list(&f, FolderRole::Inbox, None).len(), 3);
    // A deleted tombstone cannot be archived.
    let two = id(&f, &f.inbox, 2);
    f.db.execute(
        "UPDATE messages SET remote_state = 'deleted' WHERE id = ?1",
        [&two],
    )
    .unwrap();
    assert_eq!(
        code(f.service.archive_messages(&[two])),
        ErrorCode::NotFound
    );
    // Local Sent copies are not in `messages` and are refused the same way.
    assert_eq!(
        code(f.service.archive_messages(&["local-copy".into()])),
        ErrorCode::NotFound
    );
}

#[test]
fn archive_batches_are_deduplicated_and_limited() {
    let mut f = setup(false);
    let one = id(&f, &f.inbox, 1);
    f.service
        .archive_messages(&[one.clone(), one.clone()])
        .unwrap();
    let rows: i64 =
        f.db.query_row("SELECT COUNT(*) FROM message_archives", [], |r| r.get(0))
            .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(code(f.service.archive_messages(&[])), ErrorCode::Validation);
    let many: Vec<String> = (0..501).map(|n| format!("id-{n}")).collect();
    assert_eq!(
        code(f.service.archive_messages(&many)),
        ErrorCode::Validation
    );
    assert_eq!(
        code(f.service.unarchive_messages(&[])),
        ErrorCode::Validation
    );
}

#[test]
fn unarchive_restores_the_inbox_row() {
    let mut f = setup(false);
    let one = id(&f, &f.inbox, 1);
    f.service
        .archive_messages(std::slice::from_ref(&one))
        .unwrap();
    f.service
        .unarchive_messages(std::slice::from_ref(&one))
        .unwrap();
    assert_eq!(list(&f, FolderRole::Inbox, None).len(), 3);
    assert!(list(&f, FolderRole::Inbox, Some(ArchiveScope::Only)).is_empty());
    assert!(!f.service.get_email(&one).unwrap().summary.archived);
}

#[test]
fn archive_survives_a_uidvalidity_reset() {
    let mut f = setup(false);
    f.service.archive_messages(&[id(&f, &f.inbox, 1)]).unwrap();
    let reset = SyncTarget {
        uid_validity: 11,
        ..f.inbox.clone()
    };
    f.service
        .ensure_generation(&reset, "2026-09-18")
        .unwrap()
        .unwrap();
    f.service
        .store_message_batch(&reset, &[record(1, false), record(2, true)], 2)
        .unwrap()
        .unwrap();
    assert_eq!(list(&f, FolderRole::Inbox, None), ["Subject 2"]);
    assert_eq!(
        list(&f, FolderRole::Inbox, Some(ArchiveScope::Only)),
        ["Subject 1"]
    );
}

#[test]
fn mail_without_a_message_id_is_keyed_by_its_local_id() {
    let mut f = setup(false);
    f.db.execute(
        "UPDATE messages SET message_id_header = NULL WHERE uid = 1",
        [],
    )
    .unwrap();
    f.service.archive_messages(&[id(&f, &f.inbox, 1)]).unwrap();
    assert_eq!(
        list(&f, FolderRole::Inbox, Some(ArchiveScope::Only)),
        ["Subject 1"]
    );
    // After a reset the rebuilt row has a new local id and returns to the inbox.
    let reset = SyncTarget {
        uid_validity: 11,
        ..f.inbox.clone()
    };
    f.service
        .ensure_generation(&reset, "2026-09-18")
        .unwrap()
        .unwrap();
    let mut rebuilt = record(1, false);
    rebuilt.message_id_header = None;
    f.service
        .store_message_batch(&reset, &[rebuilt], 1)
        .unwrap()
        .unwrap();
    assert_eq!(list(&f, FolderRole::Inbox, None), ["Subject 1"]);
    assert!(list(&f, FolderRole::Inbox, Some(ArchiveScope::Only)).is_empty());
}

#[test]
fn a_repointed_mailbox_source_drops_the_archive() {
    let mut f = setup(false);
    f.service.archive_messages(&[id(&f, &f.inbox, 1)]).unwrap();
    f.db.execute(
        "UPDATE message_archives SET mailbox_source_id = 'old-source'",
        [],
    )
    .unwrap();
    assert_eq!(list(&f, FolderRole::Inbox, None).len(), 3);
    assert!(list(&f, FolderRole::Inbox, Some(ArchiveScope::Only)).is_empty());
}

#[test]
fn a_new_message_is_not_archived_and_other_folders_ignore_the_key() {
    let mut f = setup(true);
    f.service.archive_messages(&[id(&f, &f.inbox, 1)]).unwrap();
    f.service
        .store_message_batch(&f.inbox, &[record(4, false)], 4)
        .unwrap()
        .unwrap();
    assert_eq!(
        list(&f, FolderRole::Inbox, Some(ArchiveScope::Only)),
        ["Subject 1"]
    );
    // The same Message-ID in the trash is not archived there.
    f.db.execute(
        "UPDATE messages SET message_id_header = 'message-1@example.test' WHERE uid = 8",
        [],
    )
    .unwrap();
    assert_eq!(list(&f, FolderRole::Trash, None), ["Subject 8"]);
}

#[test]
fn displayed_unread_subtracts_archived_unread_mail() {
    let mut f = setup(false);
    let raw = |f: &Fixture| -> i64 {
        f.db.query_row(
            "SELECT unread_count FROM folders WHERE id = ?1",
            [&f.inbox.folder_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(inbox_folder(&f).unread_count, 2);
    f.service
        .archive_messages(&[id(&f, &f.inbox, 1), id(&f, &f.inbox, 2)])
        .unwrap();
    let folder = inbox_folder(&f);
    assert_eq!(
        (
            folder.unread_count,
            folder.archived_count,
            folder.archived_unread_count
        ),
        (1, 2, 1)
    );
    assert_eq!(raw(&f), 2);
    // A server count from the next sync is stored raw and still reduced.
    f.db.execute("UPDATE folders SET unread_count = 5", [])
        .unwrap();
    assert_eq!(inbox_folder(&f).unread_count, 4);
    // Never below zero.
    f.db.execute("UPDATE folders SET unread_count = 0", [])
        .unwrap();
    assert_eq!(inbox_folder(&f).unread_count, 0);
    f.service
        .unarchive_messages(&[id(&f, &f.inbox, 1)])
        .unwrap();
    assert_eq!(inbox_folder(&f).archived_unread_count, 0);
}

#[test]
fn search_finds_archived_mail_and_marks_it() {
    let mut f = setup(false);
    f.service.archive_messages(&[id(&f, &f.inbox, 1)]).unwrap();
    let page = f
        .service
        .list_emails(&EmailListQuery {
            text: Some("Subject".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.items.len(), 3);
    let archived: Vec<_> = page
        .items
        .iter()
        .filter(|e| e.archived)
        .map(|e| e.subject.clone())
        .collect();
    assert_eq!(archived, ["Subject 1"]);
}

#[test]
fn partial_archive_skips_non_inbox_and_missing_ids_and_restores_only_what_it_changed() {
    let mut f = setup(true);
    let one = id(&f, &f.inbox, 1);
    let two = id(&f, &f.inbox, 2);
    let trashed = id(&f, &f.trash, 8);
    let local = format!("local-sent:{}", Uuid::new_v4());
    let change = f
        .service
        .change_archive(
            &[
                trashed.clone(),
                one.clone(),
                local.clone(),
                one.clone(),
                two.clone(),
                "missing".into(),
            ],
            true,
            true,
        )
        .unwrap();
    assert_eq!(change.skipped_ids, vec![trashed, local, "missing".into()]);
    assert_eq!(change.processed_count, 2);
    assert_eq!(change.entries.len(), 2);
    assert_eq!(list(&f, FolderRole::Inbox, None), vec!["Subject 3"]);
    // A message archived earlier is untouched by restoring this batch.
    let three = id(&f, &f.inbox, 3);
    f.service.archive_messages(&[three]).unwrap();
    f.service.restore_archive(&change.entries).unwrap();
    let mut inbox = list(&f, FolderRole::Inbox, None);
    inbox.sort();
    assert_eq!(inbox, vec!["Subject 1", "Subject 2"]);
}

#[test]
fn partial_archive_with_nothing_usable_is_not_found_and_strict_is_unchanged() {
    let mut f = setup(true);
    let one = id(&f, &f.inbox, 1);
    let trashed = id(&f, &f.trash, 8);
    assert_eq!(
        f.service
            .change_archive(&[trashed.clone(), "missing".into()], true, true)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        f.service
            .change_archive(&[one.clone(), trashed], true, false)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    let rows: i64 =
        f.db.query_row("SELECT COUNT(*) FROM message_archives", [], |r| r.get(0))
            .unwrap();
    assert_eq!(rows, 0);
    let strict = f.service.change_archive(&[one], true, false).unwrap();
    assert!(strict.skipped_ids.is_empty());
}
