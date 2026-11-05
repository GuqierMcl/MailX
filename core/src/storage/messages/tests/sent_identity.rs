//! Reconciliation order, provider evidence, and upgrades of already-cached Sent mail.

use super::*;
use crate::send::types::{ResolveUnknownRequest, ServerReplyInfo, UnknownResolution};

fn qq_account(f: &Fixture) {
    // Selection uses the actual endpoint, including manually configured accounts.
    f.db.execute(
        "UPDATE mail_connections SET host = 'imap.qq.com' WHERE account_id = ?1 AND protocol = 'imap'",
        [&f.account.id],
    )
    .unwrap();
}

fn qq_policy() -> &'static str {
    crate::providers::for_imap_host("imap.qq.com").policy_key()
}

fn rewritten(uid: u32, original: &str) -> MessageRecord {
    let raw = format!(
        "From: me@example.test\r\nTo: a@example.test\r\nSubject: Same subject\r\n\
         Message-ID: <server-{uid}@qq.com>\r\nX-OQ-MSGID: <{original}>\r\n\r\n"
    );
    crate::sync::headers::parse_header(
        &crate::imap::RemoteHeader {
            uid,
            flags: vec![],
            internal_date: None,
            size: Some(500),
            header: raw.into_bytes(),
        },
        chrono::Utc::now(),
        crate::providers::for_imap_host("imap.qq.com"),
    )
}

fn copy(f: &Fixture, n: u32) -> String {
    local_copy(f, n, "2026-11-04T05:00:00.000Z", "Same subject", 0)
}

fn awaiting(f: &Fixture, n: u32, state: &str) {
    copy(f, n);
    f.db.execute(
        "UPDATE outbox SET state = ?2, accepted_at = NULL WHERE id = ?1",
        params![local_id(n), state],
    )
    .unwrap();
    crate::storage::search_index::refresh_local(&f.db, &local_id(n)).unwrap();
}

fn matched(f: &Fixture, n: u32) -> bool {
    f.service
        .get_outbox(&local_id(n))
        .unwrap()
        .server_copy_message_id
        .is_some()
}

fn accept(f: &Fixture, n: u32) {
    f.service
        .outbox_accept(
            &local_id(n),
            &ServerReplyInfo {
                code: 250,
                text: "accepted".into(),
            },
            chrono::Utc::now(),
        )
        .unwrap()
        .unwrap();
}

fn pending(f: &Fixture, target: &SyncTarget) -> Vec<u32> {
    f.service
        .sent_identity_refresh_uids(
            &target.account_id,
            &target.mailbox_source_id,
            &target.folder_id,
            qq_policy(),
        )
        .unwrap()
}

#[test]
fn provider_original_id_replaces_the_local_copy_and_removes_its_search_entry() {
    let mut f = setup();
    qq_account(&f);
    let local = copy(&f, 1);
    put_sent(&mut f, &[rewritten(1, "local-1@mailx.test")]);

    assert!(matched(&f, 1));
    let page = f.service.list_emails(&sent_query(&f)).unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(!page.items[0].local_only);
    assert_ne!(page.items[0].id, local);
    let (held, released): (bool, bool) = f
        .db
        .query_row(
            "SELECT snapshot_hash IS NOT NULL, snapshot_released_at IS NOT NULL FROM outbox WHERE id = ?1",
            [local_id(1)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((held, released), (false, true));
    let indexed: bool =
        f.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM search_docs WHERE email_id = ?1)",
            [&local],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!indexed);
}

#[test]
fn server_copy_arriving_before_smtp_acceptance_is_reconciled_on_acceptance() {
    for provider_alias in [false, true] {
        let mut f = setup();
        if provider_alias {
            qq_account(&f);
        }
        awaiting(&f, 1, "sending");
        let remote = if provider_alias {
            rewritten(1, "local-1@mailx.test")
        } else {
            sent_copy("local-1@mailx.test", 1)
        };
        put_sent(&mut f, &[remote]);
        assert!(!matched(&f, 1));
        accept(&f, 1);
        assert!(matched(&f, 1), "provider_alias={provider_alias}");
        assert_eq!(listed(&f, &sent_query(&f)).len(), 1);
    }
}

#[test]
fn unknown_keeps_its_snapshot_until_the_user_marks_it_sent_then_reconciles() {
    let mut f = setup();
    qq_account(&f);
    awaiting(&f, 1, "unknown");
    put_sent(&mut f, &[rewritten(1, "local-1@mailx.test")]);
    assert!(!matched(&f, 1));
    let held: bool =
        f.db.query_row(
            "SELECT snapshot_hash IS NOT NULL FROM outbox WHERE id = ?1",
            [local_id(1)],
            |row| row.get(0),
        )
        .unwrap();
    assert!(held);
    f.service
        .resolve_unknown(
            &ResolveUnknownRequest {
                id: local_id(1),
                resolution: UnknownResolution::MarkSent,
                client_mutation_id: Uuid::new_v4().to_string(),
            },
            chrono::Utc::now(),
        )
        .unwrap()
        .unwrap();
    assert!(matched(&f, 1));
    assert_eq!(listed(&f, &sent_query(&f)).len(), 1);
}

#[test]
fn native_message_id_takes_precedence_over_a_conflicting_provider_alias() {
    let mut f = setup();
    qq_account(&f);
    copy(&f, 1);
    copy(&f, 2);
    let mut remote = rewritten(1, "local-2@mailx.test");
    remote.message_id_header = Some("local-1@mailx.test".into());
    put_sent(&mut f, &[remote]);
    assert!(matched(&f, 1));
    assert!(!matched(&f, 2));
    assert_eq!(listed(&f, &sent_query(&f)).len(), 2);
}

#[test]
fn aliases_require_the_current_provider_policy_and_the_frozen_sender() {
    for bad in ["unknown-endpoint", "old-policy", "wrong-sender"] {
        let mut f = setup();
        if bad != "unknown-endpoint" {
            qq_account(&f);
        }
        copy(&f, 1);
        let mut remote = rewritten(1, "local-1@mailx.test");
        if bad == "old-policy" {
            remote.sent_identity_policy = "obsolete-policy";
        }
        if bad == "wrong-sender" {
            remote.from_address = "another@example.test".into();
        }
        put_sent(&mut f, &[remote]);
        assert!(!matched(&f, 1), "{bad}");
        assert_eq!(listed(&f, &sent_query(&f)).len(), 2, "{bad}");
    }
}

#[test]
fn late_acceptance_does_not_match_an_invisible_or_unrelated_server_copy() {
    for hidden in [
        "inbox",
        "old-generation",
        "old-source",
        "deleted",
        "pending-delete",
        "other-account",
    ] {
        let mut f = setup();
        qq_account(&f);
        awaiting(&f, 1, "sending");
        let remote = rewritten(1, "local-1@mailx.test");
        if hidden == "inbox" {
            put(&mut f, &[remote]);
        } else {
            put_sent(&mut f, &[remote]);
        }
        let id = message_id(&f, 1);
        let sent = target_for(&f, "Sent", 10);
        match hidden {
            "old-generation" => {
                f.service
                    .ensure_generation(
                        &SyncTarget {
                            uid_validity: 11,
                            ..sent.clone()
                        },
                        "2026-10-01",
                    )
                    .unwrap();
            }
            "old-source" => {
                f.db.execute(
                    "UPDATE accounts SET mailbox_source_id = 'replacement-source' WHERE id = ?1",
                    [&f.account.id],
                )
                .unwrap();
            }
            "deleted" => {
                f.service.mark_deleted(&sent, &[1]).unwrap();
            }
            "pending-delete" => {
                f.db.execute(
                    "INSERT INTO message_deletions(message_id, batch_id, account_id, mailbox_source_id, folder_id, uid_validity, uid, permanent, due_at, created_at)
                     SELECT id, 'batch', account_id, mailbox_source_id, folder_id, uid_validity, uid, 1, 't', 't' FROM messages WHERE id = ?1",
                    [&id],
                ).unwrap();
            }
            "other-account" => {
                let other = add_account(&mut f.service).0;
                f.db.execute(
                    "UPDATE outbox SET account_id = ?2 WHERE id = ?1",
                    params![local_id(1), other.id],
                )
                .unwrap();
            }
            _ => {}
        }
        accept(&f, 1);
        assert!(!matched(&f, 1), "{hidden}");
    }
}

#[test]
fn identity_backfill_preserves_flags_body_and_existing_message_identity() {
    let mut f = setup();
    qq_account(&f);
    let local = copy(&f, 1);
    let mut old = sent_copy("server-1@qq.com", 1);
    old.from_address = "me@example.test".into();
    put_sent(&mut f, &[old]);
    let target = target_for(&f, "Sent", 10);
    let id = message_id(&f, 1);
    f.db.execute(
        "UPDATE messages SET sent_identity_policy = '', preview = 'cached preview', body_state = 'ready', has_attachment = 1 WHERE id = ?1",
        [&id],
    ).unwrap();
    f.service.set_message_flag(&id, Flag::Seen, true).unwrap();
    f.service
        .set_message_flag(&id, Flag::Flagged, true)
        .unwrap();
    assert_eq!(pending(&f, &target), [1]);

    let mut remote = rewritten(1, "local-1@mailx.test");
    remote.subject = "Must not replace cached metadata".into();
    f.service
        .store_sent_identity_batch(&target, &[remote], &[1], qq_policy())
        .unwrap()
        .unwrap();
    assert!(matched(&f, 1));
    assert!(!listed(&f, &sent_query(&f)).contains(&local));
    assert_eq!(message_id(&f, 1), id);
    assert_eq!(flags_of(&f, &id), (true, true));
    let cached: (String, String, String, bool) =
        f.db.query_row(
            "SELECT subject, preview, body_state, has_attachment FROM messages WHERE id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        cached,
        (
            "Subject 1".into(),
            "cached preview".into(),
            "ready".into(),
            true
        )
    );
    assert_eq!(count(&f.db, "flag_changes"), 2);
    assert!(pending(&f, &target).is_empty());
}

#[test]
fn identity_backfill_tombstones_a_requested_uid_absent_from_a_successful_fetch() {
    let mut f = setup();
    qq_account(&f);
    copy(&f, 1);
    put_sent(&mut f, &[sent_copy("server-1@qq.com", 1)]);
    let target = target_for(&f, "Sent", 10);
    f.db.execute("UPDATE messages SET sent_identity_policy = ''", [])
        .unwrap();
    assert_eq!(pending(&f, &target), [1]);
    f.service
        .store_sent_identity_batch(&target, &[], &[1], qq_policy())
        .unwrap()
        .unwrap();
    assert!(pending(&f, &target).is_empty());
    assert!(!matched(&f, 1));
    let page = f.service.list_emails(&sent_query(&f)).unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.items[0].local_only);
}

#[test]
fn identity_backfill_is_bounded_and_its_progress_survives_reopening() {
    let mut f = setup();
    qq_account(&f);
    let records: Vec<_> = (1..=201)
        .map(|uid| sent_copy(&format!("unmatched-{uid}@qq.com"), uid))
        .collect();
    put_sent(&mut f, &records);
    let target = target_for(&f, "Sent", 10);
    f.db.execute("UPDATE messages SET sent_identity_policy = ''", [])
        .unwrap();
    let first = pending(&f, &target);
    assert_eq!(first.len(), 200);
    let refreshed: Vec<_> = first
        .iter()
        .map(|uid| rewritten(*uid, &format!("not-sent-{uid}@example.test")))
        .collect();
    f.service
        .store_sent_identity_batch(&target, &refreshed, &first, qq_policy())
        .unwrap()
        .unwrap();
    let remaining = pending(&f, &target);
    assert_eq!(remaining.len(), 1);
    assert!(!first.contains(&remaining[0]));
    let directory = f.directory.path().to_owned();
    drop(f.service);
    let service = AccountService::open(&directory, f.secrets.clone()).unwrap();
    assert_eq!(
        service
            .sent_identity_refresh_uids(
                &target.account_id,
                &target.mailbox_source_id,
                &target.folder_id,
                qq_policy(),
            )
            .unwrap(),
        remaining
    );
}

#[test]
fn a_late_identity_fetch_cannot_update_a_replaced_uid_generation() {
    let mut f = setup();
    qq_account(&f);
    copy(&f, 1);
    put_sent(&mut f, &[sent_copy("server-1@qq.com", 1)]);
    let old = target_for(&f, "Sent", 10);
    let current = SyncTarget {
        uid_validity: 11,
        ..old.clone()
    };
    f.service.ensure_generation(&current, "2026-10-01").unwrap();
    f.service
        .store_message_batch(&current, &[sent_copy("replacement@qq.com", 1)], 1)
        .unwrap();
    assert!(
        f.service
            .store_sent_identity_batch(
                &old,
                &[rewritten(1, "local-1@mailx.test")],
                &[1],
                qq_policy()
            )
            .unwrap()
            .is_none()
    );
    assert!(!matched(&f, 1));
    let identity: Option<String> = f.db.query_row(
        "SELECT original_message_id FROM messages WHERE folder_id = ?1 AND uid_validity = 11 AND uid = 1",
        [&old.folder_id], |row| row.get(0),
    ).unwrap();
    assert!(identity.is_none());
}

#[test]
fn reopening_reconciles_accepted_copies_against_headers_cached_by_older_builds() {
    let mut f = setup();
    put_sent(&mut f, &[sent_copy("local-1@mailx.test", 1)]);
    let local = copy(&f, 1);
    assert_eq!(listed(&f, &sent_query(&f)).len(), 2);
    let directory = f.directory.path().to_owned();
    drop(f.service);
    for _ in 0..2 {
        let service = AccountService::open(&directory, f.secrets.clone()).unwrap();
        let page = service
            .list_emails(&EmailListQuery {
                folder_role: Some(FolderRole::Sent),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(!page.items[0].local_only);
        assert_ne!(page.items[0].id, local);
        assert!(
            service
                .get_outbox(&local_id(1))
                .unwrap()
                .server_copy_message_id
                .is_some()
        );
    }
}

#[test]
fn undoing_a_deletion_reconciles_identity_that_finished_refreshing_while_hidden() {
    let mut f = setup();
    qq_account(&f);
    copy(&f, 1);
    let mut old = sent_copy("server-1@qq.com", 1);
    old.from_address = "me@example.test".into();
    put_sent(&mut f, &[old]);
    let target = target_for(&f, "Sent", 10);
    let id = message_id(&f, 1);
    let deletion = f
        .service
        .schedule_deletion(
            &[id],
            true,
            chrono::Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    // This fetch was already in flight when the user asked to delete the row.
    f.service
        .store_sent_identity_batch(
            &target,
            &[rewritten(1, "local-1@mailx.test")],
            &[1],
            qq_policy(),
        )
        .unwrap()
        .unwrap();
    assert!(!matched(&f, 1));
    f.service.cancel_deletion(&deletion.batch_id).unwrap();
    assert!(pending(&f, &target).is_empty());
    assert_eq!(listed(&f, &sent_query(&f)).len(), 2);
    assert_eq!(
        f.service
            .reconcile_sent_folder(
                &target.account_id,
                &target.mailbox_source_id,
                &target.folder_id,
            )
            .unwrap(),
        Some(1)
    );
    assert!(matched(&f, 1));
    assert_eq!(listed(&f, &sent_query(&f)).len(), 1);
}

#[test]
fn a_folder_newly_recognized_as_sent_reconciles_its_already_current_headers() {
    let mut f = setup();
    qq_account(&f);
    copy(&f, 1);
    let folders = |role| [folder("INBOX", FolderRole::Inbox), folder("Sent", role)];
    f.service
        .store_folders(
            &f.account.id,
            &f.account.mailbox_source_id,
            &folders(FolderRole::Other),
        )
        .unwrap();
    put_sent(&mut f, &[rewritten(1, "local-1@mailx.test")]);
    assert!(!matched(&f, 1));
    f.service
        .store_folders(
            &f.account.id,
            &f.account.mailbox_source_id,
            &folders(FolderRole::Sent),
        )
        .unwrap();
    let target = target_for(&f, "Sent", 10);
    assert!(pending(&f, &target).is_empty());
    assert_eq!(
        f.service
            .reconcile_sent_folder(
                &target.account_id,
                &target.mailbox_source_id,
                &target.folder_id,
            )
            .unwrap(),
        Some(1)
    );
    assert!(matched(&f, 1));
}

#[test]
fn a_sent_folder_round_does_not_reconcile_another_folders_cached_copy() {
    let mut f = setup();
    qq_account(&f);
    f.service
        .store_folders(
            &f.account.id,
            &f.account.mailbox_source_id,
            &[
                folder("INBOX", FolderRole::Inbox),
                folder("Sent", FolderRole::Sent),
                folder("Other Sent", FolderRole::Sent),
            ],
        )
        .unwrap();
    let other = target_for(&f, "Other Sent", 10);
    f.service.ensure_generation(&other, "2026-09-20").unwrap();
    f.service
        .store_message_batch(&other, &[rewritten(99, "local-1@mailx.test")], 99)
        .unwrap();
    copy(&f, 1);
    assert!(!matched(&f, 1));

    // Neither a new header, identity refresh, nor an unchanged-status round in
    // this folder may use potentially stale cache belonging to the other one.
    put_sent(&mut f, &[sent_copy("unrelated@example.test", 1)]);
    let target = target_for(&f, "Sent", 10);
    assert!(!matched(&f, 1));
    f.service
        .store_sent_identity_batch(
            &target,
            &[rewritten(1, "unrelated@example.test")],
            &[1],
            qq_policy(),
        )
        .unwrap();
    assert!(!matched(&f, 1));
    assert_eq!(
        f.service
            .reconcile_sent_folder(
                &target.account_id,
                &target.mailbox_source_id,
                &target.folder_id,
            )
            .unwrap(),
        Some(0)
    );
    assert!(!matched(&f, 1));
    assert_eq!(
        f.service
            .reconcile_sent_folder(
                &other.account_id,
                &other.mailbox_source_id,
                &other.folder_id,
            )
            .unwrap(),
        Some(1)
    );
    assert!(matched(&f, 1));
}
