use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::{
    accounts::{
        AccountService,
        sync_plan::tests::{config, connection_update},
        types::Account,
    },
    imap::FolderStatus,
    mail::FolderRole,
    secrets::test_support::MemorySecrets,
    sync::folders::FolderRecord,
};

struct Fixture {
    service: AccountService,
    db: Connection,
    directory: TempDir,
    secrets: Arc<MemorySecrets>,
    account: Account,
    target: SyncTarget,
}

fn folder(name: &str, role: FolderRole) -> FolderRecord {
    FolderRecord {
        remote_name: name.into(),
        parent_remote_name: None,
        name: name.into(),
        delimiter: Some("/".into()),
        role,
        selectable: true,
        attributes: vec![],
        status: Some(FolderStatus {
            messages: 0,
            unseen: 0,
            uid_next: Some(1),
            uid_validity: Some(10),
        }),
    }
}

fn add_account(service: &mut AccountService) -> (Account, SyncTarget) {
    let account = service.create(serde_json::from_value(json!({ "requestId": Uuid::new_v4().to_string(), "config": config(json!({"kind":"direct"})) })).unwrap()).unwrap();
    service
        .store_folders(
            &account.id,
            &account.mailbox_source_id,
            &[
                folder("INBOX", FolderRole::Inbox),
                folder("Sent", FolderRole::Sent),
            ],
        )
        .unwrap();
    let inbox = service
        .sync_folders(&account.id, &account.mailbox_source_id)
        .unwrap()
        .remove(0);
    let target = SyncTarget {
        account_id: account.id.clone(),
        mailbox_source_id: account.mailbox_source_id.clone(),
        folder_id: inbox.0,
        uid_validity: 10,
    };
    (account, target)
}

fn setup() -> Fixture {
    let directory = TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let mut service = AccountService::open(directory.path(), secrets.clone()).unwrap();
    let (account, target) = add_account(&mut service);
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    service
        .ensure_generation(&target, "2026-09-18")
        .unwrap()
        .unwrap();
    Fixture {
        service,
        db,
        directory,
        secrets,
        account,
        target,
    }
}

fn record(uid: u32) -> MessageRecord {
    MessageRecord {
        uid,
        message_id_header: Some(format!("message-{uid}@example.test")),
        subject: format!("Subject {uid}"),
        from_name: Some("林女士".into()),
        from_address: "sender@example.test".into(),
        to_addresses: vec![MailAddress {
            name: Some("收件人".into()),
            address: "recipient@example.test".into(),
        }],
        cc_addresses: vec![MailAddress {
            name: None,
            address: "cc@example.test".into(),
        }],
        reply_to_addresses: vec![MailAddress {
            name: None,
            address: "reply@example.test".into(),
        }],
        received_at: "2026-10-18T09:00:00.000Z".into(),
        sent_at: Some("2026-10-18T08:00:00.000Z".into()),
        size: Some(400),
        is_read: false,
        is_flagged: false,
        flags: vec![],
        has_attachment: false,
    }
}

fn inbox_query() -> EmailListQuery {
    EmailListQuery {
        folder_role: Some(FolderRole::Inbox),
        ..Default::default()
    }
}
fn count(db: &Connection, table: &str) -> u32 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
fn put(fixture: &mut Fixture, records: &[MessageRecord]) {
    assert!(
        fixture
            .service
            .store_message_batch(
                &fixture.target,
                records,
                records.iter().map(|record| record.uid).max().unwrap_or(0)
            )
            .unwrap()
            .is_some()
    );
}

#[test]
fn repeated_uid_keeps_local_id_and_reopens_with_header_only_detail() {
    let mut f = setup();
    put(&mut f, &[record(5)]);
    let first = f
        .service
        .list_emails(&inbox_query())
        .unwrap()
        .items
        .remove(0);
    let mut updated = record(5);
    updated.subject = "Updated".into();
    updated.is_read = true;
    put(&mut f, &[updated]);
    let second = f.service.get_email(&first.id).unwrap();
    assert_eq!(second.summary.id, first.id);
    assert_eq!(second.summary.subject, "Updated");
    assert!(second.summary.is_read);
    assert!(second.summary.preview.is_empty());
    assert_eq!(second.summary.body_state, BodyState::Missing);
    assert!(!second.summary.pending_changes);
    assert_eq!(second.to[0].address, "recipient@example.test");
    assert_eq!(second.cc[0].address, "cc@example.test");
    assert_eq!(second.reply_to[0].address, "reply@example.test");
    assert!(second.body.is_none() && second.attachments.is_empty());
    assert_eq!(count(&f.db, "messages"), 1);
    let Fixture {
        service,
        db,
        directory,
        secrets,
        ..
    } = f;
    drop(service);
    drop(db);
    let reopened = AccountService::open(directory.path(), secrets).unwrap();
    assert_eq!(reopened.get_email(&first.id).unwrap(), second);
}

fn cache_body_for_query(f: &mut Fixture) -> String {
    put(f, &[record(7)]);
    let id = f.service.list_emails(&inbox_query()).unwrap().items[0]
        .id
        .clone();
    f.db.execute(
        "INSERT INTO message_bodies(message_id, raw_sha256, raw_size, plain_text,
         sanitized_html, sanitizer_version, links, remote_images_blocked, truncated, fetched_at)
         VALUES (?1, ?2, 20, '仅正文纯文本', '<p>PRIVATE_HTML_SENTINEL</p>', 1,
         '[\"https://example.invalid/private-link\"]', 3, 1, '2026-10-19')",
        params![id, "a".repeat(64)],
    )
    .unwrap();
    for (index, name, inline) in [
        (1, Some("报告.pdf"), false),
        (2, Some("logo.png"), true),
        (3, None, false),
    ] {
        f.db.execute(
            "INSERT INTO message_parts(message_id, part_index, content_id, filename, mime_type, size, is_inline)
             VALUES (?1, ?2, ?3, ?4, ?5, 123, ?6)",
            params![id, index, inline.then_some("logo"), name, if inline { "image/png" } else { "application/pdf" }, inline],
        ).unwrap();
    }
    f.db.execute(
        "UPDATE messages SET preview = '摘要🙂', body_state = 'ready', body_failure = NULL WHERE id = ?1",
        [&id],
    ).unwrap();
    id
}

#[test]
fn ready_queries_return_safe_body_metadata_preview_and_non_inline_attachments() {
    let mut f = setup();
    let id = cache_body_for_query(&mut f);
    let page = f.service.list_emails(&inbox_query()).unwrap();
    assert_eq!(page.items[0].preview, "摘要🙂");
    assert_eq!(page.items[0].body_state, BodyState::Ready);
    assert!(!page.coverage.bodies_complete);
    let detail = f.service.get_email(&id).unwrap();
    let body = detail.body.as_ref().unwrap();
    assert_eq!(body.text.as_deref(), Some("仅正文纯文本"));
    assert!(body.has_html && body.truncated);
    assert_eq!(body.remote_images_blocked, 3);
    assert!(detail.body_failure.is_none());
    assert_eq!(detail.attachments.len(), 2);
    assert_eq!(detail.attachments[0].filename, "报告.pdf");
    assert_eq!(detail.attachments[0].mime_type, "application/pdf");
    assert_eq!(detail.attachments[0].size, 123);
    assert_eq!(detail.attachments[0].id, format!("{id}:1"));
    assert_eq!(detail.attachments[1].filename, "未命名附件");
    assert!(detail.attachments.iter().all(|part| !part.is_inline));
    let wire = serde_json::to_value(&detail).unwrap();
    assert!(wire["body"].get("html").is_none());
    let wire = wire.to_string();
    assert!(!wire.contains("PRIVATE_HTML_SENTINEL"));
    assert!(!wire.contains("private-link"));
    assert!(!wire.contains(&"a".repeat(64)));
    assert!(!wire.contains("raw_sha256"));

    let mut changed_header = record(7);
    changed_header.subject = "Updated header".into();
    put(&mut f, &[changed_header]);
    let updated = f.service.get_email(&id).unwrap();
    assert_eq!(updated.summary.subject, "Updated header");
    assert_eq!(updated.summary.preview, "摘要🙂");
    assert_eq!(updated.body, detail.body);
    assert_eq!(updated.attachments, detail.attachments);
}

#[test]
fn failed_and_in_progress_queries_never_expose_previous_ready_content() {
    use crate::mail::BodyFailureCode;

    let mut f = setup();
    let id = cache_body_for_query(&mut f);
    for (reason, expected) in [
        ("tooLarge", BodyFailureCode::TooLarge),
        ("stale", BodyFailureCode::Stale),
        ("network", BodyFailureCode::Network),
        ("authentication", BodyFailureCode::Authentication),
        ("protocol", BodyFailureCode::Protocol),
        ("storage", BodyFailureCode::Storage),
    ] {
        f.db.execute(
            "UPDATE messages SET body_state = 'failed', body_failure = ?1",
            [reason],
        )
        .unwrap();
        let detail = f.service.get_email(&id).unwrap();
        assert_eq!(detail.body_failure, Some(expected));
        assert_eq!(detail.summary.body_state, BodyState::Failed);
        assert!(detail.body.is_none() && detail.attachments.is_empty());
    }
    for (state, expected) in [
        ("missing", BodyState::Missing),
        ("fetching", BodyState::Fetching),
    ] {
        f.db.execute(
            "UPDATE messages SET body_state = ?1, body_failure = NULL",
            [state],
        )
        .unwrap();
        let detail = f.service.get_email(&id).unwrap();
        assert_eq!(detail.summary.body_state, expected);
        assert!(detail.body_failure.is_none());
        assert!(detail.body.is_none() && detail.attachments.is_empty());
    }
    f.db.execute(
        "UPDATE messages SET body_state = 'failed', body_failure = 'untrusted server text'",
        [],
    )
    .unwrap();
    let error = f.service.get_email(&id).unwrap_err();
    assert_eq!(error.code, ErrorCode::StorageIncompatible);
    assert!(!serde_json::to_string(&error).unwrap().contains("untrusted"));
}

#[test]
fn ready_body_cache_does_not_bypass_current_message_visibility() {
    for change in [
        "UPDATE accounts SET mailbox_source_id = 'new-source'",
        "UPDATE folders SET selectable = 0",
        "UPDATE folder_sync_cursors SET uid_validity = 11",
        "UPDATE messages SET remote_state = 'deleted'",
        "UPDATE messages SET remote_state = 'stale'",
    ] {
        let mut f = setup();
        let id = cache_body_for_query(&mut f);
        f.db.execute(change, []).unwrap();
        assert_eq!(
            f.service.get_email(&id).unwrap_err().code,
            ErrorCode::NotFound,
            "{change}"
        );
        assert!(
            f.service
                .list_emails(&inbox_query())
                .unwrap()
                .items
                .is_empty(),
            "{change}"
        );
    }
}

#[test]
fn same_generation_keeps_window_and_progress_and_new_generation_hides_old_rows() {
    let mut f = setup();
    put(&mut f, &[record(20)]);
    f.service.finish_backfill(&f.target, false).unwrap();
    let old = f
        .service
        .list_emails(&inbox_query())
        .unwrap()
        .items
        .remove(0);
    let cursor = f
        .service
        .ensure_generation(&f.target, "2026-10-01")
        .unwrap()
        .unwrap();
    assert_eq!(cursor.window_start, "2026-09-18");
    assert_eq!(cursor.highest_uid, 20);
    assert!(cursor.backfill_done);
    let new_target = SyncTarget {
        uid_validity: 11,
        ..f.target.clone()
    };
    let new_cursor = f
        .service
        .ensure_generation(&new_target, "2026-10-01")
        .unwrap()
        .unwrap();
    assert_eq!(new_cursor.highest_uid, 0);
    assert_eq!(new_cursor.window_start, "2026-10-01");
    assert!(!new_cursor.backfill_done);
    assert!(
        f.service
            .list_emails(&inbox_query())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        f.service.get_email(&old.id).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        f.db.query_row("SELECT remote_state FROM messages", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "stale"
    );
    assert!(
        f.service
            .store_message_batch(&new_target, &[record(20)], 20)
            .unwrap()
            .is_some()
    );
    let new = f
        .service
        .list_emails(&inbox_query())
        .unwrap()
        .items
        .remove(0);
    assert_ne!(new.id, old.id);
    assert_eq!(count(&f.db, "messages"), 2);
}

fn assert_rejected(service: &mut AccountService, target: &SyncTarget) {
    assert!(
        service
            .store_message_batch(target, &[record(99)], 99)
            .unwrap()
            .is_none()
    );
    assert!(service.begin_backfill(target, true).unwrap().is_none());
    assert!(service.finish_backfill(target, true).unwrap().is_none());
    assert!(service.mark_deleted(target, &[1]).unwrap().is_none());
    assert!(
        service
            .apply_flags(target, &[(1, vec!["\\Seen".into()])])
            .unwrap()
            .is_none()
    );
    assert!(service.cached_uids(target).unwrap().is_empty());
}

#[test]
fn every_write_rechecks_account_source_folder_and_generation() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let (_, other) = add_account(&mut f.service);
    for target in [
        SyncTarget {
            account_id: other.account_id,
            ..f.target.clone()
        },
        SyncTarget {
            mailbox_source_id: Uuid::new_v4().to_string(),
            ..f.target.clone()
        },
        SyncTarget {
            folder_id: other.folder_id,
            ..f.target.clone()
        },
        SyncTarget {
            folder_id: Uuid::new_v4().to_string(),
            ..f.target.clone()
        },
        SyncTarget {
            uid_validity: 11,
            ..f.target.clone()
        },
    ] {
        assert_rejected(&mut f.service, &target);
        if target.uid_validity == f.target.uid_validity {
            assert!(
                f.service
                    .ensure_generation(&target, "2026-10-01")
                    .unwrap()
                    .is_none()
            );
            assert!(
                f.service
                    .update_folder_counts(
                        &target.account_id,
                        &target.mailbox_source_id,
                        &target.folder_id,
                        99,
                        100
                    )
                    .unwrap()
                    .is_none()
            );
        }
    }
    assert_eq!(count(&f.db, "messages"), 1);
    let saved = f
        .service
        .list_emails(&inbox_query())
        .unwrap()
        .items
        .remove(0);
    assert!(!saved.is_read);
    assert_eq!(
        f.service
            .ensure_generation(&f.target, "ignored")
            .unwrap()
            .unwrap()
            .highest_uid,
        1
    );
}

#[test]
fn source_change_hides_cache_before_folder_refresh_and_rejects_late_work() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let id = f
        .service
        .list_emails(&inbox_query())
        .unwrap()
        .items
        .remove(0)
        .id;
    let mut config = config(json!({"kind":"direct"}));
    config["imap"]["host"] = json!("new.example.test");
    let updated = f
        .service
        .update_connection(connection_update(&f.account.id, f.account.revision, config))
        .unwrap();
    assert_ne!(updated.mailbox_source_id, f.target.mailbox_source_id);
    assert!(
        f.service
            .list_emails(&inbox_query())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        f.service.get_email(&id).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_rejected(&mut f.service, &f.target);
    assert!(
        f.service
            .ensure_generation(&f.target, "2026-10-01")
            .unwrap()
            .is_none()
    );
    assert!(
        f.service
            .sync_folders(&f.account.id, &f.target.mailbox_source_id)
            .unwrap()
            .is_empty()
    );
    f.service
        .store_folders(
            &updated.id,
            &updated.mailbox_source_id,
            &[folder("INBOX", FolderRole::Inbox)],
        )
        .unwrap();
    assert_eq!(count(&f.db, "messages"), 0);
    assert_eq!(count(&f.db, "folder_sync_cursors"), 0);
}

#[test]
fn removed_folder_cascades_mail_and_cursor_and_composite_identity_is_enforced() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    assert!(
        f.db.execute(
            "UPDATE messages SET account_id = ?1",
            [Uuid::new_v4().to_string()]
        )
        .is_err()
    );
    assert!(
        f.db.execute(
            "UPDATE messages SET mailbox_source_id = ?1",
            [Uuid::new_v4().to_string()]
        )
        .is_err()
    );
    f.service
        .store_folders(
            &f.account.id,
            &f.account.mailbox_source_id,
            &[folder("Sent", FolderRole::Sent)],
        )
        .unwrap();
    assert_eq!(count(&f.db, "messages"), 0);
    assert_eq!(count(&f.db, "folder_sync_cursors"), 0);
    assert_rejected(&mut f.service, &f.target);
}

#[test]
fn deleting_given_uids_and_applying_flags_do_not_touch_other_rows() {
    let mut f = setup();
    put(&mut f, &[record(1), record(2), record(3)]);
    let deleted_id = f
        .service
        .list_emails(&inbox_query())
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.subject == "Subject 2")
        .unwrap()
        .id;
    f.service.mark_deleted(&f.target, &[2, 999]).unwrap();
    f.service
        .apply_flags(
            &f.target,
            &[
                (
                    1,
                    vec!["\\sEeN".into(), "\\FLAGGED".into(), "$custom".into()],
                ),
                (2, vec!["\\Seen".into()]),
            ],
        )
        .unwrap();
    let page = f.service.list_emails(&inbox_query()).unwrap();
    assert_eq!(page.items.len(), 2);
    let first = page
        .items
        .iter()
        .find(|item| item.subject == "Subject 1")
        .unwrap();
    let third = page
        .items
        .iter()
        .find(|item| item.subject == "Subject 3")
        .unwrap();
    assert!(first.is_read && first.is_starred);
    assert!(!third.is_read && !third.is_starred);
    assert_eq!(
        f.service.get_email(&deleted_id).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        f.db.query_row("SELECT flags FROM messages WHERE uid = 1", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        // Server spelling is kept; the order is normalized so equal sets compare equal.
        r#"["$custom","\FLAGGED","\sEeN"]"#.replace("\\", "\\\\")
    );
    assert!(
        !f.db
            .query_row("SELECT is_read FROM messages WHERE uid = 2", [], |row| row
                .get::<_, bool>(
                0
            ))
            .unwrap()
    );
    f.service.apply_flags(&f.target, &[(1, vec![])]).unwrap();
    assert!(!f.service.get_email(&first.id).unwrap().summary.is_starred);
}

#[test]
fn message_batch_and_cursor_roll_back_together_on_write_failure() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    f.db.execute_batch("CREATE TRIGGER fail_second_message BEFORE INSERT ON messages WHEN NEW.uid = 3 BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
    assert!(
        f.service
            .store_message_batch(&f.target, &[record(2), record(3)], 3)
            .is_err()
    );
    assert_eq!(count(&f.db, "messages"), 1);
    assert_eq!(
        f.service
            .ensure_generation(&f.target, "ignored")
            .unwrap()
            .unwrap()
            .highest_uid,
        1
    );
    f.db.execute_batch("DROP TRIGGER fail_second_message;")
        .unwrap();
    // Fail after all message INSERTs, at the cursor update itself.
    f.db.execute_batch("CREATE TRIGGER fail_cursor BEFORE UPDATE ON folder_sync_cursors WHEN NEW.highest_uid = 3 BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
    assert!(
        f.service
            .store_message_batch(&f.target, &[record(2), record(3)], 3)
            .is_err()
    );
    assert_eq!(count(&f.db, "messages"), 1);
    assert_eq!(
        f.service
            .ensure_generation(&f.target, "ignored")
            .unwrap()
            .unwrap()
            .highest_uid,
        1
    );
    f.db.execute_batch("DROP TRIGGER fail_cursor;").unwrap();
    put(&mut f, &[record(2), record(3)]);
    assert_eq!(count(&f.db, "messages"), 3);
}

#[test]
fn folder_counts_update_only_the_current_identity() {
    let mut f = setup();
    assert!(
        f.service
            .update_folder_counts(
                &f.target.account_id,
                &f.target.mailbox_source_id,
                &f.target.folder_id,
                4,
                9
            )
            .unwrap()
            .is_some()
    );
    let folders = f.service.list_folders(&f.account.id).unwrap();
    let inbox = folders
        .iter()
        .find(|folder| folder.id == f.target.folder_id)
        .unwrap();
    assert_eq!((inbox.unread_count, inbox.total_count), (4, 9));
    assert!(
        f.service
            .update_folder_counts(
                &f.target.account_id,
                "old-source",
                &f.target.folder_id,
                0,
                0
            )
            .unwrap()
            .is_none()
    );
    let folders = f.service.list_folders(&f.account.id).unwrap();
    let inbox = folders
        .iter()
        .find(|folder| folder.id == f.target.folder_id)
        .unwrap();
    assert_eq!((inbox.unread_count, inbox.total_count), (4, 9));
}

#[test]
fn list_filters_accounts_folder_role_flags_text_and_normalized_time() {
    let mut f = setup();
    let mut a = record(1);
    a.subject = "税务 100%_\\ note".into();
    a.received_at = "2026-10-18T01:00:00.000Z".into();
    a.has_attachment = true;
    let mut b = record(2);
    b.from_name = Some("Second Sender".into());
    b.from_address = "other@example.test".into();
    b.is_read = true;
    put(&mut f, &[a, b]);
    let (_, second) = add_account(&mut f.service);
    f.service.ensure_generation(&second, "2026-10-01").unwrap();
    f.service
        .store_message_batch(&second, &[record(3)], 3)
        .unwrap();
    let subjects = |query: EmailListQuery| {
        f.service
            .list_emails(&query)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.subject)
            .collect::<Vec<_>>()
    };
    assert_eq!(subjects(inbox_query()).len(), 3);
    let scoped = EmailListQuery {
        account_ids: Some(vec![f.account.id.clone()]),
        ..inbox_query()
    };
    assert_eq!(subjects(scoped.clone()).len(), 2);
    assert!(
        subjects(EmailListQuery {
            account_ids: Some(vec![]),
            ..inbox_query()
        })
        .is_empty()
    );
    assert_eq!(
        subjects(EmailListQuery {
            folder_id: Some(f.target.folder_id.clone()),
            ..Default::default()
        })
        .len(),
        2
    );
    assert!(
        subjects(EmailListQuery {
            folder_role: Some(FolderRole::Sent),
            ..Default::default()
        })
        .is_empty()
    );
    assert_eq!(
        subjects(EmailListQuery {
            unread: Some(true),
            ..scoped.clone()
        }),
        ["税务 100%_\\ note"]
    );
    assert_eq!(
        subjects(EmailListQuery {
            unread: Some(false),
            ..scoped.clone()
        }),
        ["Subject 2"]
    );
    assert_eq!(
        subjects(EmailListQuery {
            has_attachment: Some(true),
            ..scoped.clone()
        }),
        ["税务 100%_\\ note"]
    );
    assert_eq!(
        subjects(EmailListQuery {
            has_attachment: Some(false),
            ..scoped.clone()
        }),
        ["Subject 2"]
    );
    for text in ["  100%_\\  ", "税务", "林女士", "sender@example"] {
        assert_eq!(
            subjects(EmailListQuery {
                text: Some(text.into()),
                ..scoped.clone()
            }),
            ["税务 100%_\\ note"]
        );
    }
    for text in ["second sender", "other@"] {
        assert_eq!(
            subjects(EmailListQuery {
                text: Some(text.into()),
                ..scoped.clone()
            }),
            ["Subject 2"]
        );
    }
    assert!(
        subjects(EmailListQuery {
            text: Some("%' OR 1=1 --".into()),
            ..scoped.clone()
        })
        .is_empty()
    );
    assert_eq!(
        subjects(EmailListQuery {
            text: Some("   ".into()),
            ..scoped.clone()
        })
        .len(),
        2
    );
    assert_eq!(
        subjects(EmailListQuery {
            received_after: Some("2026-10-18T09:00:00+08:00".into()),
            received_before: Some("2026-10-18T17:00:00+08:00".into()),
            ..scoped
        }),
        ["税务 100%_\\ note"]
    );
}

#[test]
fn same_timestamp_pages_are_stable_without_duplicates_or_missing_rows() {
    let mut f = setup();
    let rows: Vec<_> = (1..=205).map(record).collect();
    put(&mut f, &rows);
    let mut query = EmailListQuery {
        limit: Some(37),
        ..inbox_query()
    };
    let mut ids = vec![];
    loop {
        let page = f.service.list_emails(&query).unwrap();
        assert!(page.items.len() <= 37);
        ids.extend(page.items.into_iter().map(|item| item.id));
        match page.next_cursor {
            Some(cursor) => query.cursor = Some(cursor),
            None => break,
        }
    }
    assert_eq!(ids.len(), 205);
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]));
    assert_eq!(
        f.service.list_emails(&inbox_query()).unwrap().items.len(),
        50
    );
    assert_eq!(
        f.service
            .list_emails(&EmailListQuery {
                limit: Some(200),
                ..inbox_query()
            })
            .unwrap()
            .items
            .len(),
        200
    );
}

#[test]
fn coverage_uses_every_scoped_folder_and_tracks_failed_or_capped_backfill() {
    let mut f = setup();
    assert!(
        !f.service
            .list_emails(&inbox_query())
            .unwrap()
            .coverage
            .headers_complete
    );
    f.service.finish_backfill(&f.target, false).unwrap();
    let complete = f.service.list_emails(&inbox_query()).unwrap().coverage;
    assert!(complete.headers_complete);
    assert!(!complete.window_capped);
    assert!(!complete.bodies_complete);
    assert_eq!(complete.synced_after.as_deref(), Some("2026-09-18"));
    assert_eq!(complete.synced_before, None);
    assert!(
        !f.service
            .list_emails(&EmailListQuery::default())
            .unwrap()
            .coverage
            .headers_complete
    );
    assert!(
        !f.service
            .list_emails(&EmailListQuery {
                folder_role: Some(FolderRole::Sent),
                ..Default::default()
            })
            .unwrap()
            .coverage
            .headers_complete
    );
    f.service.begin_backfill(&f.target, false).unwrap();
    assert!(
        !f.service
            .list_emails(&inbox_query())
            .unwrap()
            .coverage
            .headers_complete
    );
    f.service.finish_backfill(&f.target, true).unwrap();
    let capped = f.service.list_emails(&inbox_query()).unwrap().coverage;
    assert!(!capped.headers_complete);
    assert!(capped.window_capped);
    f.service.finish_backfill(&f.target, false).unwrap();
    let (_, second) = add_account(&mut f.service);
    assert!(
        !f.service
            .list_emails(&inbox_query())
            .unwrap()
            .coverage
            .headers_complete
    );
    f.service.ensure_generation(&second, "2026-10-01").unwrap();
    f.service.finish_backfill(&second, false).unwrap();
    let both = f
        .service
        .list_emails(&EmailListQuery {
            text: Some("no-match".into()),
            ..inbox_query()
        })
        .unwrap()
        .coverage;
    assert!(both.headers_complete);
    assert_eq!(both.synced_after.as_deref(), Some("2026-10-01"));
    f.service.begin_backfill(&second, false).unwrap();
    let first_only = EmailListQuery {
        account_ids: Some(vec![f.account.id.clone()]),
        ..inbox_query()
    };
    assert!(
        f.service
            .list_emails(&first_only)
            .unwrap()
            .coverage
            .headers_complete
    );
    assert!(
        !f.service
            .list_emails(&EmailListQuery {
                account_ids: Some(vec![]),
                ..inbox_query()
            })
            .unwrap()
            .coverage
            .headers_complete
    );
}

fn target_for(f: &Fixture, remote_name: &str, uid_validity: u32) -> SyncTarget {
    let (folder_id, _) = f
        .service
        .sync_folders(&f.account.id, &f.account.mailbox_source_id)
        .unwrap()
        .into_iter()
        .find(|(_, name)| name == remote_name)
        .unwrap();
    SyncTarget {
        folder_id,
        uid_validity,
        ..f.target.clone()
    }
}

#[test]
fn sync_targets_cover_all_selectable_roles_in_stable_order_without_requiring_inbox() {
    let mut f = setup();
    let mut container = folder("Projects", FolderRole::Other);
    container.selectable = false;
    container.status = None;
    let mut child = folder("Projects/中文 \"项目\"", FolderRole::Other);
    child.parent_remote_name = Some("Projects".into());
    let records = [
        folder("Trash", FolderRole::Trash),
        folder("Sent", FolderRole::Sent),
        folder("INBOX", FolderRole::Inbox),
        folder("Drafts", FolderRole::Drafts),
        folder("Archive", FolderRole::Archive),
        folder("Junk", FolderRole::Junk),
        child,
        container,
    ];
    f.service
        .store_folders(&f.account.id, &f.account.mailbox_source_id, &records)
        .unwrap();
    let targets = f
        .service
        .sync_folders(&f.account.id, &f.account.mailbox_source_id)
        .unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "INBOX",
            "Archive",
            "Drafts",
            "Junk",
            "Projects/中文 \"项目\"",
            "Sent",
            "Trash"
        ]
    );
    let no_inbox: Vec<_> = records
        .into_iter()
        .filter(|record| record.role != FolderRole::Inbox)
        .rev()
        .collect();
    f.service
        .store_folders(&f.account.id, &f.account.mailbox_source_id, &no_inbox)
        .unwrap();
    assert_eq!(
        f.service
            .sync_folders(&f.account.id, &f.account.mailbox_source_id)
            .unwrap(),
        targets[1..]
    );
    assert!(
        f.service
            .sync_folders(&f.account.id, "previous-source")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn list_only_refresh_keeps_known_counts_and_persists_unknown_until_status_succeeds() {
    let mut f = setup();
    f.service
        .update_folder_counts(
            &f.account.id,
            &f.account.mailbox_source_id,
            &f.target.folder_id,
            4,
            9,
        )
        .unwrap();
    let mut records = [
        folder("INBOX", FolderRole::Inbox),
        folder("Sent", FolderRole::Sent),
        folder("New", FolderRole::Other),
    ];
    for record in &mut records {
        record.status = None;
    }
    f.service
        .store_folders(&f.account.id, &f.account.mailbox_source_id, &records)
        .unwrap();
    drop(f.service);
    f.service = AccountService::open(f.directory.path(), f.secrets.clone()).unwrap();
    let folders = f.service.list_folders(&f.account.id).unwrap();
    let inbox = folders
        .iter()
        .find(|folder| folder.role == FolderRole::Inbox)
        .unwrap();
    assert_eq!(
        (inbox.unread_count, inbox.total_count, inbox.counts_known),
        (4, 9, true)
    );
    let new = folders.iter().find(|folder| folder.name == "New").unwrap();
    assert!(!new.counts_known);
    assert_eq!((new.unread_count, new.total_count), (0, 0));
    let (uid_next, uid_validity): (Option<u32>, Option<u32>) =
        f.db.query_row(
            "SELECT uid_next, uid_validity FROM folders WHERE id = ?1",
            [&f.target.folder_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((uid_next, uid_validity), (Some(1), Some(10)));
    f.service
        .update_folder_counts(&f.account.id, &f.account.mailbox_source_id, &new.id, 0, 0)
        .unwrap();
    f.service
        .store_folders(&f.account.id, &f.account.mailbox_source_id, &records)
        .unwrap();
    drop(f.service);
    f.service = AccountService::open(f.directory.path(), f.secrets.clone()).unwrap();
    let after = f.service.list_folders(&f.account.id).unwrap();
    assert!(
        after
            .iter()
            .find(|folder| folder.id == new.id)
            .unwrap()
            .counts_known
    );
}

#[test]
fn identical_uids_and_message_ids_stay_independent_across_folder_lifecycles() {
    let mut f = setup();
    let sent = target_for(&f, "Sent", 10);
    f.service.ensure_generation(&sent, "2026-09-20").unwrap();
    let original = record(7);
    put(&mut f, std::slice::from_ref(&original));
    f.service
        .store_message_batch(&sent, std::slice::from_ref(&original), 7)
        .unwrap();
    let both = f
        .service
        .list_emails(&EmailListQuery::default())
        .unwrap()
        .items;
    assert_eq!(both.len(), 2);
    assert_ne!(both[0].id, both[1].id);
    let sent_mail = both
        .iter()
        .find(|mail| mail.folder_id == sent.folder_id)
        .unwrap();
    f.service
        .apply_flags(&f.target, &[(7, vec!["\\Seen".into(), "\\Flagged".into()])])
        .unwrap();
    assert!(!f.service.get_email(&sent_mail.id).unwrap().summary.is_read);
    assert!(
        !f.service
            .get_email(&sent_mail.id)
            .unwrap()
            .summary
            .is_starred
    );
    // A move converges as source deletion plus destination insertion; the
    // matching Message-ID does not merge their independent folder identities.
    f.service.mark_deleted(&f.target, &[7]).unwrap();
    assert!(
        f.service
            .list_emails(&inbox_query())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        f.service
            .get_email(&sent_mail.id)
            .unwrap()
            .summary
            .folder_id,
        sent.folder_id
    );
    put(&mut f, &[record(7)]);
    let next_sent = SyncTarget {
        uid_validity: 11,
        ..sent.clone()
    };
    f.service
        .ensure_generation(&next_sent, "2026-10-01")
        .unwrap();
    assert_eq!(
        f.service.get_email(&sent_mail.id).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        f.service.list_emails(&inbox_query()).unwrap().items.len(),
        1
    );
    assert_eq!(f.service.cached_uids(&f.target).unwrap().len(), 1);
    assert_rejected(&mut f.service, &sent);
    f.service
        .store_folders(
            &f.account.id,
            &f.account.mailbox_source_id,
            &[
                folder("INBOX", FolderRole::Inbox),
                folder("Renamed", FolderRole::Sent),
            ],
        )
        .unwrap();
    let renamed = target_for(&f, "Renamed", 11);
    assert_ne!(renamed.folder_id, sent.folder_id);
    assert_eq!(count(&f.db, "messages"), 1);
    assert_eq!(count(&f.db, "folder_sync_cursors"), 1);
    assert_rejected(&mut f.service, &next_sent);
}

#[test]
fn non_inbox_filters_and_keyset_pages_never_include_matching_mail_from_other_folders() {
    let mut f = setup();
    let sent = target_for(&f, "Sent", 10);
    f.service.ensure_generation(&sent, "2026-09-18").unwrap();
    let rows: Vec<_> = (1..=31)
        .map(|uid| {
            let mut message = record(uid);
            message.subject = "项目 100%".into();
            message.is_read = uid % 2 == 0;
            message.has_attachment = uid % 2 != 0;
            message
        })
        .collect();
    put(&mut f, &rows);
    f.service.store_message_batch(&sent, &rows, 31).unwrap();
    let scoped = EmailListQuery {
        folder_id: Some(sent.folder_id.clone()),
        unread: Some(true),
        has_attachment: Some(true),
        text: Some(" 100% ".into()),
        received_after: Some("2026-10-18T00:00:00Z".into()),
        received_before: Some("2026-10-19T00:00:00Z".into()),
        limit: Some(3),
        ..Default::default()
    };
    let mut query = scoped.clone();
    let mut ids = vec![];
    loop {
        let page = f.service.list_emails(&query).unwrap();
        assert!(
            page.items
                .iter()
                .all(|mail| mail.folder_id == sent.folder_id)
        );
        ids.extend(page.items.into_iter().map(|mail| mail.id));
        match page.next_cursor {
            Some(cursor) => query.cursor = Some(cursor),
            None => break,
        }
    }
    assert_eq!(ids.len(), 16);
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]));
    let by_role = f
        .service
        .list_emails(&EmailListQuery {
            folder_id: None,
            folder_role: Some(FolderRole::Sent),
            limit: Some(200),
            ..scoped
        })
        .unwrap();
    assert_eq!(
        by_role
            .items
            .into_iter()
            .map(|mail| mail.id)
            .collect::<Vec<_>>(),
        ids
    );
    assert!(
        f.service
            .list_emails(&inbox_query())
            .unwrap()
            .items
            .iter()
            .all(|mail| mail.folder_id == f.target.folder_id)
    );
}

#[test]
fn noselect_hides_cached_mail_rejects_late_writes_and_excludes_coverage() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    f.service.finish_backfill(&f.target, true).unwrap();
    let old_id = f
        .service
        .list_emails(&inbox_query())
        .unwrap()
        .items
        .remove(0)
        .id;
    let sent = target_for(&f, "Sent", 10);
    f.service.ensure_generation(&sent, "2026-09-20").unwrap();
    f.service.finish_backfill(&sent, false).unwrap();
    let mut container = folder("INBOX", FolderRole::Inbox);
    container.selectable = false;
    container.status = None;
    let mut child = folder("INBOX/Child", FolderRole::Other);
    child.parent_remote_name = Some("INBOX".into());
    f.service
        .store_folders(
            &f.account.id,
            &f.account.mailbox_source_id,
            &[container, child, folder("Sent", FolderRole::Sent)],
        )
        .unwrap();
    let child_target = target_for(&f, "INBOX/Child", 10);
    f.service
        .ensure_generation(&child_target, "2026-09-25")
        .unwrap();
    f.service
        .store_message_batch(&child_target, &[record(1)], 1)
        .unwrap();
    f.service.finish_backfill(&child_target, false).unwrap();
    let folders = f.service.list_folders(&f.account.id).unwrap();
    let container = folders
        .iter()
        .find(|folder| folder.id == f.target.folder_id)
        .unwrap();
    assert!(!container.selectable);
    assert!(!container.counts_known);
    assert!(
        f.service
            .list_emails(&inbox_query())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        f.service.get_email(&old_id).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_rejected(&mut f.service, &f.target);
    assert!(
        f.service
            .ensure_generation(&f.target, "2026-10-01")
            .unwrap()
            .is_none()
    );
    assert!(
        f.service
            .update_folder_counts(
                &f.account.id,
                &f.account.mailbox_source_id,
                &f.target.folder_id,
                1,
                1
            )
            .unwrap()
            .is_none()
    );
    let page = f.service.list_emails(&EmailListQuery::default()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].folder_id, child_target.folder_id);
    assert!(page.coverage.headers_complete);
    assert!(!page.coverage.window_capped);
    assert_eq!(page.coverage.synced_after.as_deref(), Some("2026-09-25"));
    let none = f.service.list_emails(&inbox_query()).unwrap().coverage;
    assert!(!none.headers_complete);
    assert!(!none.window_capped);
    assert_eq!(none.synced_after, None);
}

#[test]
fn invalid_external_query_values_return_validation() {
    let f = setup();
    let bad_cursor = |value| URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap());
    let invalid = [
        EmailListQuery {
            limit: Some(0),
            ..Default::default()
        },
        EmailListQuery {
            limit: Some(201),
            ..Default::default()
        },
        EmailListQuery {
            account_ids: Some(vec!["invalid".into()]),
            ..Default::default()
        },
        EmailListQuery {
            folder_id: Some("invalid".into()),
            ..Default::default()
        },
        EmailListQuery {
            folder_id: Some(f.target.folder_id.clone()),
            ..inbox_query()
        },
        EmailListQuery {
            received_after: Some("2026-10-18".into()),
            ..Default::default()
        },
        EmailListQuery {
            received_before: Some("not-a-date".into()),
            ..Default::default()
        },
        EmailListQuery {
            cursor: Some("!invalid".into()),
            ..Default::default()
        },
        EmailListQuery {
            cursor: Some(URL_SAFE_NO_PAD.encode("not-json")),
            ..Default::default()
        },
        EmailListQuery {
            cursor: Some(bad_cursor(
                json!({"r":"2026-10-18T09:00:00Z", "i":"invalid"}),
            )),
            ..Default::default()
        },
        EmailListQuery {
            cursor: Some(bad_cursor(
                json!({"r":"invalid", "i":Uuid::new_v4().to_string()}),
            )),
            ..Default::default()
        },
        EmailListQuery {
            cursor: Some(bad_cursor(
                json!({"r":"2026-10-18T09:00:00Z", "i":Uuid::new_v4().to_string(), "extra":true}),
            )),
            ..Default::default()
        },
    ];
    for query in invalid {
        assert_eq!(
            f.service.list_emails(&query).unwrap_err().code,
            ErrorCode::Validation
        );
    }
    assert_eq!(
        f.service.get_email("invalid").unwrap_err().code,
        ErrorCode::Validation
    );
    assert_eq!(
        f.service
            .get_email(&Uuid::new_v4().to_string())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[test]
fn writes_report_only_real_changes() {
    let mut f = setup();
    let target = f.target.clone();
    let account_id = f.account.id.clone();
    let source = f.account.mailbox_source_id.clone();
    let counts = |service: &mut AccountService, unread, total| {
        service
            .update_folder_counts(&account_id, &source, &target.folder_id, unread, total)
            .unwrap()
    };
    assert_eq!(counts(&mut f.service, 3, 10), Some(1));
    assert_eq!(
        counts(&mut f.service, 3, 10),
        Some(0),
        "same counts are no change"
    );
    assert_eq!(counts(&mut f.service, 2, 10), Some(1));

    put(&mut f, &[record(1), record(2)]);
    let flags = |order: [&str; 2]| vec![(1, order.iter().map(|flag| (*flag).to_owned()).collect())];
    assert_eq!(
        f.service
            .apply_flags(&target, &flags(["\\Seen", "\\Flagged"]))
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        f.service
            .apply_flags(&target, &flags(["\\Flagged", "\\Seen"]))
            .unwrap(),
        Some(0),
        "the same flags in another order are no change"
    );

    assert_eq!(f.service.mark_deleted(&target, &[2, 99]).unwrap(), Some(1));
    assert_eq!(
        f.service.mark_deleted(&target, &[2]).unwrap(),
        Some(0),
        "already deleted"
    );

    assert_eq!(f.service.finish_backfill(&target, false).unwrap(), Some(1));
    assert_eq!(f.service.finish_backfill(&target, false).unwrap(), Some(0));
    assert_eq!(f.service.begin_backfill(&target, false).unwrap(), Some(1));

    let created = f
        .service
        .ensure_generation(&target, "2026-09-18")
        .unwrap()
        .unwrap();
    assert!(!created.created, "an existing generation is reused");
    let replaced = SyncTarget {
        uid_validity: 11,
        ..target
    };
    let created = f
        .service
        .ensure_generation(&replaced, "2026-09-18")
        .unwrap()
        .unwrap();
    assert!(created.created, "a new UIDVALIDITY starts a new generation");
}

#[test]
fn a_revived_message_keeps_the_attachment_flag_its_downloaded_body_established() {
    let mut fixture = setup();
    let mut first = record(1);
    first.has_attachment = true;
    put(&mut fixture, &[first]);
    let id: String = fixture
        .db
        .query_row("SELECT id FROM messages WHERE uid = 1", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    let flag = |fixture: &Fixture| -> bool {
        fixture
            .db
            .query_row(
                "SELECT has_attachment FROM messages WHERE id = ?1",
                [&id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
    };
    // The body showed there is no real attachment; then the server deletes and
    // later re-lists the message, whose header still guesses "attachment".
    fixture
        .db
        .execute(
            "UPDATE messages SET body_state = 'ready', has_attachment = 0, remote_state = 'deleted' WHERE id = ?1",
            [&id],
        )
        .unwrap();
    let mut again = record(1);
    again.has_attachment = true;
    put(&mut fixture, &[again]);
    assert!(
        !flag(&fixture),
        "the header estimate overwrote the real answer"
    );

    // Without a downloaded body the new header's estimate still applies.
    fixture
        .db
        .execute(
            "UPDATE messages SET body_state = 'missing', remote_state = 'deleted' WHERE id = ?1",
            [&id],
        )
        .unwrap();
    let mut third = record(1);
    third.has_attachment = true;
    put(&mut fixture, &[third]);
    assert!(flag(&fixture));
}

// ---- local flag changes -------------------------------------------------

use crate::storage::{Flag, LocalChange};

fn message_id(f: &Fixture, uid: u32) -> String {
    f.db.query_row("SELECT id FROM messages WHERE uid = ?1", [uid], |row| {
        row.get::<_, String>(0)
    })
    .unwrap()
}

fn flags_of(f: &Fixture, id: &str) -> (bool, bool) {
    f.db.query_row(
        "SELECT is_read, is_flagged FROM messages WHERE id = ?1",
        [id],
        |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?)),
    )
    .unwrap()
}

fn server_flags(f: &mut Fixture, uid: u32, flags: &[&str]) {
    let flags: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    f.service
        .apply_flags(&f.target.clone(), &[(uid, flags)])
        .unwrap();
}

#[test]
fn a_local_change_is_visible_at_once_and_marked_as_waiting() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let id = message_id(&f, 1);
    assert_eq!(flags_of(&f, &id), (false, false));
    assert_eq!(
        f.service.set_message_flag(&id, Flag::Seen, true).unwrap(),
        LocalChange::Queued
    );
    assert_eq!(flags_of(&f, &id), (true, false));
    let listed = f.service.list_emails(&inbox_query()).unwrap().items;
    assert!(listed[0].is_read && listed[0].pending_changes);
    assert!(f.service.get_email(&id).unwrap().summary.pending_changes);
    // Setting what it already is records nothing.
    assert_eq!(
        f.service.set_message_flag(&id, Flag::Seen, true).unwrap(),
        LocalChange::Unchanged
    );
    assert_eq!(count(&f.db, "flag_changes"), 1);
}

#[test]
fn toggling_twice_keeps_one_row_with_the_latest_wish() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let id = message_id(&f, 1);
    f.service
        .set_message_flag(&id, Flag::Flagged, true)
        .unwrap();
    f.service
        .set_message_flag(&id, Flag::Flagged, false)
        .unwrap();
    assert_eq!(count(&f.db, "flag_changes"), 1);
    let wanted: bool =
        f.db.query_row("SELECT wanted FROM flag_changes", [], |row| row.get(0))
            .unwrap();
    assert!(!wanted);
    assert_eq!(flags_of(&f, &id), (false, false));
}

#[test]
fn a_sync_round_never_undoes_a_flag_that_is_still_waiting() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let id = message_id(&f, 1);
    f.service.set_message_flag(&id, Flag::Seen, true).unwrap();
    f.service
        .set_message_flag(&id, Flag::Flagged, true)
        .unwrap();
    // The server still reports the old values: neither toggle snaps back.
    server_flags(&mut f, 1, &[]);
    assert_eq!(flags_of(&f, &id), (true, true));
    // The raw flag list does follow the server.
    let raw: String =
        f.db.query_row("SELECT flags FROM messages WHERE id = ?1", [&id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(raw, "[]");
    // A flag with no waiting change is still taken from the server.
    f.db.execute("DELETE FROM flag_changes WHERE flag = 'seen'", [])
        .unwrap();
    server_flags(&mut f, 1, &["\\Flagged"]);
    assert_eq!(flags_of(&f, &id), (false, true));
}

#[test]
fn confirming_a_change_removes_it_unless_the_user_changed_their_mind_meanwhile() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let id = message_id(&f, 1);
    f.service.set_message_flag(&id, Flag::Seen, true).unwrap();
    // The write of "seen = true" is in flight; the user marks it unread again.
    f.service.set_message_flag(&id, Flag::Seen, false).unwrap();
    assert!(!f.service.finish_flag_change(&id, Flag::Seen, true).unwrap());
    assert_eq!(
        count(&f.db, "flag_changes"),
        1,
        "the newer wish must stay queued"
    );
    assert!(
        f.service
            .finish_flag_change(&id, Flag::Seen, false)
            .unwrap()
    );
    assert_eq!(count(&f.db, "flag_changes"), 0);
    assert!(!f.service.get_email(&id).unwrap().summary.pending_changes);
}

#[test]
fn an_unflaggable_folder_keeps_stars_on_this_device_only() {
    let mut f = setup();
    put(&mut f, &[record(1), record(2)]);
    let (one, two) = (message_id(&f, 1), message_id(&f, 2));
    f.service
        .set_message_flag(&one, Flag::Flagged, true)
        .unwrap();
    f.service.set_message_flag(&two, Flag::Seen, true).unwrap();
    assert_eq!(
        f.service
            .mark_folder_unflaggable(&f.target.folder_id)
            .unwrap(),
        1,
        "only the star moves to local-only"
    );
    let states: Vec<(String, String)> = {
        let mut statement =
            f.db.prepare("SELECT flag, state FROM flag_changes ORDER BY flag")
                .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        states,
        [
            ("flagged".into(), "local".into()),
            ("seen".into(), "pending".into())
        ]
    );
    // New stars in that folder are local-only right away; read still goes out.
    assert_eq!(
        f.service
            .set_message_flag(&two, Flag::Flagged, true)
            .unwrap(),
        LocalChange::LocalOnly
    );
    assert_eq!(
        f.service.set_message_flag(&two, Flag::Seen, false).unwrap(),
        LocalChange::Queued
    );
    // Local-only stars survive a sync that reports no star, and are not "pending".
    server_flags(&mut f, 1, &[]);
    assert_eq!(flags_of(&f, &one), (false, true));
    assert!(!f.service.get_email(&one).unwrap().summary.pending_changes);
    let waiting = f.service.pending_flag_changes(&f.account.id, 10).unwrap();
    assert!(waiting.iter().all(|change| change.flag == Flag::Seen));
}

#[test]
fn only_visible_messages_can_be_changed_and_deleted_ones_drop_their_changes() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let id = message_id(&f, 1);
    assert_eq!(
        f.service
            .set_message_flag("00000000-0000-4000-8000-000000000000", Flag::Seen, true)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    f.service.set_message_flag(&id, Flag::Seen, true).unwrap();
    f.db.execute(
        "UPDATE messages SET remote_state = 'deleted' WHERE id = ?1",
        [&id],
    )
    .unwrap();
    // A deleted message is neither editable nor sent to the server.
    assert_eq!(
        f.service
            .set_message_flag(&id, Flag::Flagged, true)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(
        f.service
            .pending_flag_changes(&f.account.id, 10)
            .unwrap()
            .is_empty()
    );
    // Removing the row removes its changes with it.
    f.db.execute("DELETE FROM messages WHERE id = ?1", [&id])
        .unwrap();
    assert_eq!(count(&f.db, "flag_changes"), 0);
}

#[test]
fn pending_changes_carry_everything_needed_to_address_the_message() {
    let mut f = setup();
    put(&mut f, &[record(7)]);
    let id = message_id(&f, 7);
    f.service.set_message_flag(&id, Flag::Seen, true).unwrap();
    let waiting = f.service.pending_flag_changes(&f.account.id, 10).unwrap();
    assert_eq!(waiting.len(), 1);
    let change = &waiting[0];
    assert_eq!(
        (
            change.message_id.as_str(),
            change.uid,
            change.uid_validity,
            change.remote_name.as_str()
        ),
        (id.as_str(), 7, 10, "INBOX")
    );
    assert!(change.wanted && change.flag == Flag::Seen && change.attempts == 0);
    f.service.retry_flag_change(&id, Flag::Seen).unwrap();
    assert_eq!(
        f.service.pending_flag_changes(&f.account.id, 10).unwrap()[0].attempts,
        1
    );
    assert!(
        f.service
            .pending_flag_changes("another-account", 10)
            .unwrap()
            .is_empty()
    );
}
