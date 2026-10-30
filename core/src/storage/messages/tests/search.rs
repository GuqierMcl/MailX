//! Full-text search through the list path: index maintenance on header
//! writes, local sent copies, and queries combined with filters and cursors.

use super::*;
use crate::storage::search_index::refresh_message;

fn indexed(f: &Fixture, id: &str) -> bool {
    f.db.query_row(
        "SELECT COUNT(*) FROM search_docs WHERE email_id = ?1",
        [id],
        |row| row.get::<_, u32>(0),
    )
    .unwrap()
        == 1
}

fn text_query(text: &str) -> EmailListQuery {
    EmailListQuery {
        text: Some(text.into()),
        ..inbox_query()
    }
}

/// Stands in for the body fetch: the production commit path is covered in
/// `storage::search_index::tests`.
fn give_body(f: &Fixture, uid: u32, text: &str) -> String {
    let id = message_id(f, uid);
    f.db.execute(
        "INSERT OR REPLACE INTO message_bodies(message_id, raw_sha256, raw_size, plain_text,
            sanitizer_version, links, remote_images_blocked, truncated, fetched_at)
         VALUES (?1, 'x', 1, ?2, 1, '[]', 0, 0, 't')",
        rusqlite::params![id, text],
    )
    .unwrap();
    f.db.execute(
        "UPDATE messages SET body_state = 'ready' WHERE id = ?1",
        [&id],
    )
    .unwrap();
    refresh_message(&f.db, &id).unwrap();
    id
}

#[test]
fn header_inserts_and_updates_maintain_the_index() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let id = message_id(&f, 1);
    assert!(indexed(&f, &id));
    assert_eq!(
        listed(&f, &text_query("recipient@example")),
        vec![id.clone()]
    );
    let mut changed = record(1);
    changed.subject = "Renamed invoice".into();
    changed.to_addresses[0].address = "new@example.test".into();
    put(&mut f, &[changed]);
    assert_eq!(listed(&f, &text_query("Renamed")), vec![id.clone()]);
    assert_eq!(listed(&f, &text_query("new@example")), vec![id.clone()]);
    assert!(listed(&f, &text_query("recipient@example")).is_empty());
    assert_eq!(count(&f.db, "search_docs"), 1);
}

#[test]
fn deleting_and_switching_generation_remove_index_rows() {
    let mut f = setup();
    put(&mut f, &[record(1), record(2)]);
    let (first, second) = (message_id(&f, 1), message_id(&f, 2));
    f.service.mark_deleted(&f.target, &[1]).unwrap();
    assert!(!indexed(&f, &first));
    assert!(indexed(&f, &second));
    let reopened = target_for(&f, "INBOX", 11);
    f.service
        .ensure_generation(&reopened, "2026-10-01")
        .unwrap();
    assert!(!indexed(&f, &second));
    assert_eq!(count(&f.db, "search_docs"), 0);
}

#[test]
fn removing_a_folder_clears_the_index() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    assert_eq!(count(&f.db, "search_docs"), 1);
    f.db.execute("DELETE FROM folders", []).unwrap();
    assert_eq!(count(&f.db, "search_docs"), 0);
    assert_eq!(count(&f.db, "search_fts"), 0);
}

#[test]
fn local_copy_lifecycle_follows_the_sent_list_rules() {
    let mut f = setup();
    let copy = local_copy(&f, 1, "2026-10-18T10:00:00.000Z", "Offer letter", 0);
    let sent = sent_query(&f);
    let query = |text: &str| EmailListQuery {
        text: Some(text.into()),
        ..sent.clone()
    };
    assert_eq!(listed(&f, &query("Offer")), vec![copy.clone()]);
    assert_eq!(listed(&f, &query("Local body")), vec![copy.clone()]);
    assert_eq!(listed(&f, &query("a@example")), vec![copy.clone()]);
    // Not part of the inbox scope.
    assert!(listed(&f, &text_query("Offer")).is_empty());
    // A server copy with the same Message-ID replaces it.
    put_sent(&mut f, &[sent_copy("local-1@mailx.test", 5)]);
    assert!(!indexed(&f, &copy));
    // Removed from this device.
    let other = local_copy(&f, 2, "2026-10-18T10:00:00.000Z", "Second offer", 0);
    assert!(indexed(&f, &other));
    f.service
        .remove_local_copy(&other, chrono::Utc::now())
        .unwrap();
    assert!(!indexed(&f, &other));
    // Snapshot released by a non-qualifying state change.
    let third = local_copy(&f, 3, "2026-10-18T10:00:00.000Z", "Third offer", 0);
    f.db.execute(
        "UPDATE outbox SET snapshot_released_at = 't' WHERE id = ?1",
        [local_id(3)],
    )
    .unwrap();
    crate::storage::search_index::refresh_local(&f.db, &local_id(3)).unwrap();
    assert!(!indexed(&f, &third));
}

#[test]
fn queued_outbox_rows_are_never_indexed() {
    let f = setup();
    outbox_row(&f, &f.account.id, "7", "<q@example.test>", "queued");
    crate::storage::search_index::refresh_local(&f.db, &format!("{:0>36}", "7")).unwrap();
    assert_eq!(count(&f.db, "search_docs"), 0);
}

#[test]
fn invalid_text_is_a_validation_error_and_blank_text_is_no_filter() {
    let mut f = setup();
    put(&mut f, &[record(1)]);
    let many = (0..17)
        .map(|n| format!("term{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    for text in [many, "x".repeat(257)] {
        let error = f.service.list_emails(&text_query(&text)).unwrap_err();
        assert_eq!(error.code, crate::ErrorCode::Validation);
    }
    assert_eq!(listed(&f, &text_query("   ")).len(), 1);
    assert_eq!(listed(&f, &text_query("\"\"")).len(), 1);
}

#[test]
fn chinese_short_terms_match_the_body_and_mix_with_long_terms() {
    let mut f = setup();
    let mut a = record(1);
    a.subject = "Quarterly".into();
    let mut b = record(2);
    b.subject = "Quarterly".into();
    put(&mut f, &[a, b]);
    let (a, b) = (
        give_body(&f, 1, "请查收发票"),
        give_body(&f, 2, "请查收合同"),
    );
    assert_eq!(listed(&f, &text_query("发票")), vec![a.clone()]);
    assert_eq!(listed(&f, &text_query("Quarterly 合同")), vec![b.clone()]);
    assert_eq!(listed(&f, &text_query("\"请查收发票\"")), [a]);
    assert!(listed(&f, &text_query("发票 合同")).is_empty());
}

#[test]
fn text_combines_with_filters_and_keeps_the_cursor_continuous() {
    let mut f = setup();
    let rows: Vec<_> = (1..=30)
        .map(|uid| {
            let mut r = record(uid);
            r.subject = if uid % 2 == 0 {
                "match even".into()
            } else {
                "other odd".into()
            };
            r.is_read = uid % 3 == 0;
            r.has_attachment = uid % 5 == 0;
            r
        })
        .collect();
    put(&mut f, &rows);
    let mut query = EmailListQuery {
        limit: Some(4),
        ..text_query("match")
    };
    let mut ids = vec![];
    loop {
        let page = f.service.list_emails(&query).unwrap();
        ids.extend(page.items.into_iter().map(|item| item.id));
        match page.next_cursor {
            Some(cursor) => query.cursor = Some(cursor),
            None => break,
        }
    }
    assert_eq!(ids.len(), 15);
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]));
    let unread = EmailListQuery {
        unread: Some(true),
        ..text_query("match")
    };
    assert_eq!(listed(&f, &unread).len(), 10);
    let attached = EmailListQuery {
        has_attachment: Some(true),
        ..text_query("match")
    };
    assert_eq!(listed(&f, &attached).len(), 3);
}

#[test]
fn snippets_come_only_from_body_hits_and_coverage_counts_indexed_bodies() {
    let mut f = setup();
    let mut a = record(1);
    a.subject = "needle in subject".into();
    put(&mut f, &[a, record(2)]);
    let page = f.service.list_emails(&inbox_query()).unwrap();
    assert_eq!(page.coverage.indexed_bodies, 0);
    assert!(!page.coverage.bodies_complete);
    give_body(&f, 1, "plain content");
    let id = give_body(
        &f,
        2,
        "A line with the NEEDLE inside \u{E000}marker\u{E001}.",
    );
    let page = f.service.list_emails(&text_query("needle")).unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.coverage.indexed_bodies, 2);
    for item in &page.items {
        if item.id == id {
            let segments = item.match_snippet.as_ref().unwrap();
            assert!(segments.iter().any(|s| s.hit && s.text == "NEEDLE"));
            let all: String = segments.iter().map(|s| s.text.as_str()).collect();
            assert!(!all.contains('\u{E000}') && !all.contains('\u{E001}') && all.contains("line"));
        } else {
            assert!(item.match_snippet.is_none());
        }
    }
    assert!(
        f.service
            .list_emails(&inbox_query())
            .unwrap()
            .items
            .iter()
            .all(|i| i.match_snippet.is_none())
    );
}
