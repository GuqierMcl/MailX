use super::*;
use crate::mail::render::{Rendered, RenderedPart};
use crate::storage::previews::PreviewOutcome;
use tempfile::TempDir;

fn open() -> (TempDir, Storage) {
    let directory = TempDir::new().unwrap();
    let store = Storage::open(directory.path()).unwrap();
    store
        .db
        .execute_batch(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
                VALUES ('account', 1, 'Test', 'test@example.invalid', 'source', '2026-10-19', '2026-10-19');
             INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
                attributes, unread_count, total_count, synced_at)
                VALUES ('folder', 'account', 'source', 'INBOX', 'Inbox', 'inbox', 1, '[]', 1, 1, '2026-10-19');
             INSERT INTO folder_sync_cursors(folder_id, uid_validity, window_start, window_capped,
                backfill_done, highest_uid, updated_at)
                VALUES ('folder', 10, '2026-09-19', 0, 1, 7, '2026-10-19');",
        )
        .unwrap();
    (directory, store)
}

fn add_message(db: &Connection, id: &str, uid: u32, subject: &str) {
    db.execute(
        "INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
            subject, from_name, from_address, to_addresses, cc_addresses, reply_to_addresses,
            received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
         VALUES (?1, 'account', 'source', 'folder', 10, ?2, ?3, 'Alice', 'alice@example.invalid',
            '[{\"name\":\"Bob\",\"address\":\"bob@example.invalid\"}]', '[]', '[]',
            '2026-10-19', 0, 0, '[]', 0, 'present', '2026-10-19')",
        params![id, uid, subject],
    )
    .unwrap();
}

fn rendered(text: &str, filename: Option<&str>) -> Rendered {
    Rendered {
        plain_text: Some(text.into()),
        sanitized_html: None,
        links: vec![],
        remote_images_blocked: 0,
        truncated: false,
        parts: filename
            .map(|name| RenderedPart {
                part_index: 1,
                content_id: None,
                filename: Some(name.into()),
                mime_type: "application/pdf".into(),
                size: 9,
                is_inline: false,
            })
            .into_iter()
            .collect(),
    }
}

fn commit_body(store: &mut Storage, id: &str, text: &str, filename: Option<&str>) {
    let identity = store.body_identity(id).unwrap();
    let blob = store.blobs.put(text.as_bytes()).unwrap();
    store.body_begin(&identity).unwrap();
    assert!(
        store
            .body_commit(&identity, &blob, &rendered(text, filename))
            .unwrap()
    );
}

fn row(db: &Connection, id: &str) -> Option<[String; 5]> {
    db.query_row(
        "SELECT f.subject, f.people, f.preview, f.body, f.attachments
         FROM search_docs d JOIN search_fts f ON f.rowid = d.rowid WHERE d.email_id = ?1",
        [id],
        |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?]),
    )
    .optional()
    .unwrap()
}

fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn matching(db: &Connection, text: &str) -> Vec<String> {
    let terms = parse_query(text).unwrap().unwrap();
    let (sql, values) = terms.email_ids_subquery();
    let mut statement = db.prepare(&format!("{sql} ORDER BY d.email_id")).unwrap();
    statement
        .query_map(rusqlite::params_from_iter(values), |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn width_folding_maps_fullwidth_ascii_and_ideographic_space() {
    assert_eq!(fold_width("ＡＢｃ１２３！～"), "ABc123!~");
    assert_eq!(fold_width("a\u{3000}b"), "a b");
    assert_eq!(fold_width("发票 invoice"), "发票 invoice");
}

#[test]
fn query_parsing_splits_quotes_and_enforces_limits() {
    assert_eq!(parse_query("   ").unwrap(), None);
    assert_eq!(parse_query("\"\"").unwrap(), None);
    let terms = parse_query("invoice 发票 \"quarterly report\" ＡＢ")
        .unwrap()
        .unwrap();
    assert_eq!(terms.long, ["invoice", "quarterly report"]);
    assert_eq!(terms.short, ["发票", "AB"]);
    let many = (0..17)
        .map(|n| format!("t{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        parse_query(&many).unwrap_err().code,
        crate::ErrorCode::Validation
    );
    let sixteen = (0..16)
        .map(|n| format!("t{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(parse_query(&sixteen).unwrap().is_some());
    assert_eq!(
        parse_query(&"a".repeat(257)).unwrap_err().code,
        crate::ErrorCode::Validation
    );
    assert!(parse_query(&"a".repeat(256)).unwrap().is_some());
}

#[test]
fn body_is_truncated_on_a_char_boundary_and_markers_are_stripped() {
    let (_directory, mut store) = open();
    add_message(&store.db, "m1", 1, "subject");
    let text = format!("\u{E000}start\u{E001} {}", "字".repeat(30_000));
    commit_body(&mut store, "m1", &text, None);
    let indexed = row(&store.db, "m1").unwrap();
    assert!(indexed[3].starts_with("start "));
    assert!(!indexed[3].contains(MARK_START) && !indexed[3].contains(MARK_END));
    assert!(indexed[3].len() <= BODY_LIMIT);
    assert!(indexed[3].chars().all(|c| c != '\u{FFFD}'));
}

#[test]
fn index_text_is_width_folded_both_ways() {
    let (_directory, mut store) = open();
    add_message(&store.db, "m1", 1, "ＩＮＶＯＩＣＥ 2026");
    commit_body(&mut store, "m1", "Ｔｏｔａｌ ｄｕｅ", None);
    assert_eq!(matching(&store.db, "invoice"), ["m1"]);
    assert_eq!(matching(&store.db, "ＩＮＶＯＩＣＥ"), ["m1"]);
    assert_eq!(matching(&store.db, "total"), ["m1"]);
    assert_eq!(matching(&store.db, "ｔｏｔａｌ"), ["m1"]);
}

#[test]
fn body_commit_indexes_body_people_and_attachment_names() {
    let (_directory, mut store) = open();
    add_message(&store.db, "m1", 1, "Hello");
    refresh_message(&store.db, "m1").unwrap();
    let before = row(&store.db, "m1").unwrap();
    assert_eq!(before[0], "Hello");
    assert!(before[1].contains("Alice") && before[1].contains("bob@example.invalid"));
    assert_eq!(before[3], "");
    commit_body(
        &mut store,
        "m1",
        "请查收发票和合同",
        Some("contract-2026.pdf"),
    );
    let after = row(&store.db, "m1").unwrap();
    assert_eq!(after[3], "请查收发票和合同");
    assert_eq!(after[4], "contract-2026.pdf");
    assert_eq!(matching(&store.db, "contract-2026"), ["m1"]);
    assert_eq!(matching(&store.db, "发票"), ["m1"]);
    // Recommitting replaces rather than duplicates.
    commit_body(&mut store, "m1", "second", None);
    assert_eq!(count(&store.db, "search_docs"), 1);
    assert_eq!(count(&store.db, "search_fts"), 1);
    assert_eq!(row(&store.db, "m1").unwrap()[4], "");
}

#[test]
fn preview_commit_updates_the_index() {
    let (_directory, mut store) = open();
    add_message(&store.db, "m1", 1, "Hello");
    let target = store.preview_targets(&["m1".to_owned()]).unwrap().remove(0);
    store
        .preview_commit(&[PreviewOutcome {
            identity: target,
            preview: Some("partial fetched sentence".into()),
        }])
        .unwrap();
    assert_eq!(row(&store.db, "m1").unwrap()[2], "partial fetched sentence");
    assert_eq!(matching(&store.db, "fetched sentence"), ["m1"]);
}

#[test]
fn deleting_messages_and_cascades_removes_index_rows() {
    let (_directory, store) = open();
    add_message(&store.db, "m1", 1, "alpha subject");
    add_message(&store.db, "m2", 2, "beta subject");
    refresh_message(&store.db, "m1").unwrap();
    refresh_message(&store.db, "m2").unwrap();
    store
        .db
        .execute("DELETE FROM messages WHERE id = 'm1'", [])
        .unwrap();
    assert!(row(&store.db, "m1").is_none());
    assert!(row(&store.db, "m2").is_some());
    store
        .db
        .execute("DELETE FROM accounts WHERE id = 'account'", [])
        .unwrap();
    assert_eq!(count(&store.db, "search_docs"), 0);
    assert_eq!(count(&store.db, "search_fts"), 0);
}

#[test]
fn refresh_drops_rows_that_are_no_longer_present() {
    let (_directory, store) = open();
    add_message(&store.db, "m1", 1, "alpha subject");
    refresh_message(&store.db, "m1").unwrap();
    store
        .db
        .execute("UPDATE messages SET remote_state = 'deleted'", [])
        .unwrap();
    refresh_message(&store.db, "m1").unwrap();
    assert!(row(&store.db, "m1").is_none());
    remove_folder(&store.db, "folder").unwrap();
}

#[test]
fn escaping_keeps_operators_literal() {
    let (_directory, store) = open();
    for (n, subject) in [
        "say \"quoted\" words",
        "star* wildcard",
        "this AND that",
        "NEAR(a b) call",
        "dash-separated minus -value",
        "col:on value",
        "100% sure_thing back\\slash",
    ]
    .into_iter()
    .enumerate()
    {
        add_message(&store.db, &format!("m{n}"), n as u32 + 1, subject);
        refresh_message(&store.db, &format!("m{n}")).unwrap();
    }
    assert_eq!(matching(&store.db, "\"say\""), ["m0"]);
    assert_eq!(matching(&store.db, "star*"), ["m1"]);
    assert_eq!(matching(&store.db, "this AND that"), ["m2"]);
    assert_eq!(matching(&store.db, "AND"), ["m2"]);
    assert_eq!(matching(&store.db, "NEAR(a"), ["m3"]);
    assert_eq!(matching(&store.db, "minus -value"), ["m4"]);
    assert_eq!(matching(&store.db, "col:on"), ["m5"]);
    assert_eq!(matching(&store.db, "-"), ["m4"]);
    assert_eq!(matching(&store.db, "%"), ["m6"]);
    assert_eq!(matching(&store.db, "_"), ["m6"]);
    assert_eq!(matching(&store.db, "\\"), ["m6"]);
    assert_eq!(matching(&store.db, "sure_thing"), ["m6"]);
    assert!(matching(&store.db, "sureXthing").is_empty());
    // A lone `%` or `_` must not behave as a wildcard.
    assert_eq!(matching(&store.db, "10_%").len(), 0);
}

#[test]
fn short_terms_use_like_and_mix_with_match() {
    let (_directory, mut store) = open();
    add_message(&store.db, "m1", 1, "Quarterly report");
    add_message(&store.db, "m2", 2, "Quarterly plan");
    commit_body(&mut store, "m1", "请查收发票", None);
    commit_body(&mut store, "m2", "请查收合同", None);
    assert_eq!(matching(&store.db, "发票"), ["m1"]);
    assert_eq!(matching(&store.db, "Quarterly"), ["m1", "m2"]);
    assert_eq!(matching(&store.db, "Quarterly 发票"), ["m1"]);
    assert_eq!(
        matching(&store.db, "Quarterly 发票 合同"),
        Vec::<String>::new()
    );
    assert_eq!(matching(&store.db, "\"Quarterly report\""), ["m1"]);
    assert!(matching(&store.db, "\"report Quarterly\"").is_empty());
}

#[test]
fn backfill_is_batched_resumable_and_idempotent() {
    let (_directory, mut store) = open();
    for n in 0..5u32 {
        add_message(&store.db, &format!("m{n}"), n + 1, &format!("subject {n}"));
    }
    store
        .db
        .execute_batch(
            "DELETE FROM search_fts; DELETE FROM search_docs;
             UPDATE search_index_state SET backfill_done = 0, messages_cursor = '', local_done = 0;",
        )
        .unwrap();
    assert!(!store.search_index_backfill_step(2).unwrap());
    assert_eq!(count(&store.db, "search_docs"), 2);
    // Reopen mid-way: progress is stored and the rest is picked up.
    let directory = _directory;
    drop(store);
    let mut store = Storage::open(directory.path()).unwrap();
    assert_eq!(count(&store.db, "search_docs"), 2);
    let mut steps = 0;
    while !store.search_index_backfill_step(2).unwrap() {
        steps += 1;
        assert!(steps < 10);
    }
    assert_eq!(count(&store.db, "search_docs"), 5);
    assert_eq!(count(&store.db, "search_fts"), 5);
    assert_eq!(
        store.search_index_consistency().unwrap(),
        IndexConsistency::default()
    );
    assert!(store.search_index_backfill_step(2).unwrap());
}

#[test]
fn backfill_also_covers_qualifying_local_copies() {
    let (_directory, mut store) = open();
    store.db.execute_batch(LOCAL_FIXTURE).unwrap();
    store
        .db
        .execute_batch(
            "DELETE FROM search_fts; DELETE FROM search_docs;
             UPDATE search_index_state SET backfill_done = 0, messages_cursor = '', local_done = 0;",
        )
        .unwrap();
    while !store.search_index_backfill_step(10).unwrap() {}
    assert_eq!(
        row(&store.db, "local-sent:00000000-0000-4000-8000-000000000001").unwrap()[0],
        "Local subject"
    );
    assert!(row(&store.db, "local-sent:00000000-0000-4000-8000-000000000002").is_none());
    assert!(row(&store.db, "local-sent:00000000-0000-4000-8000-000000000003").is_none());
}

#[test]
fn version_mismatch_drops_the_index_and_restarts_the_backfill() {
    let (directory, mut store) = open();
    add_message(&store.db, "m1", 1, "alpha subject");
    while !store.search_index_backfill_step(10).unwrap() {}
    assert_eq!(count(&store.db, "search_docs"), 1);
    store
        .db
        .execute(
            "UPDATE search_index_state SET version = ?1",
            [INDEX_VERSION + 1],
        )
        .unwrap();
    drop(store);
    let mut store = Storage::open(directory.path()).unwrap();
    assert_eq!(count(&store.db, "search_docs"), 0);
    assert_eq!(count(&store.db, "search_fts"), 0);
    while !store.search_index_backfill_step(10).unwrap() {}
    assert_eq!(matching(&store.db, "alpha"), ["m1"]);
    let version: i64 = store
        .db
        .query_row("SELECT version FROM search_index_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, INDEX_VERSION);
}

#[test]
fn consistency_check_reports_missing_and_orphan_rows() {
    let (_directory, store) = open();
    add_message(&store.db, "m1", 1, "alpha subject");
    assert_eq!(store.search_index_consistency().unwrap().missing, 1);
    refresh_message(&store.db, "m1").unwrap();
    store
        .db
        .execute("UPDATE messages SET remote_state = 'deleted'", [])
        .unwrap();
    assert_eq!(store.search_index_consistency().unwrap().orphans, 1);
}

#[test]
fn snippets_split_on_markers_and_skip_header_only_hits() {
    let (_directory, mut store) = open();
    add_message(&store.db, "m1", 1, "Greetings");
    add_message(&store.db, "m2", 2, "needle in subject");
    commit_body(
        &mut store,
        "m1",
        &format!(
            "some long preamble text before the needle {} and an \u{E000}ending\u{E001}",
            "word ".repeat(30)
        ),
        Some("needle-notes.pdf"),
    );
    commit_body(&mut store, "m2", "plain body", None);
    let ids = vec!["m1".to_owned(), "m2".to_owned()];
    let long = parse_query("needle").unwrap().unwrap();
    let snippets = store.search_snippets(&long, &ids).unwrap();
    let segments = &snippets["m1"];
    assert!(
        segments
            .iter()
            .any(|s| s.hit && s.text.eq_ignore_ascii_case("needle"))
    );
    assert!(
        segments
            .iter()
            .all(|s| !s.text.contains(MARK_START) && !s.text.contains(MARK_END))
    );
    assert!(!snippets.contains_key("m2"));
    // Short terms are located in Rust, around the first occurrence.
    let short = parse_query("pl").unwrap().unwrap();
    let snippets = store.search_snippets(&short, &ids).unwrap();
    let segments = &snippets["m2"];
    assert_eq!(
        segments.iter().map(|s| s.text.as_str()).collect::<String>(),
        "plain body"
    );
    assert!(segments.iter().any(|s| s.hit && s.text == "pl"));
    let chinese = parse_query("发票").unwrap().unwrap();
    commit_body(
        &mut store,
        "m2",
        &format!("{}发票{}", "前".repeat(60), "后".repeat(60)),
        None,
    );
    let segments = &store.search_snippets(&chinese, &ids).unwrap()["m2"];
    assert_eq!(segments[0].text, format!("…{}", "前".repeat(40)));
    assert_eq!(
        segments[1],
        SnippetSegment {
            text: "发票".into(),
            hit: true
        }
    );
    assert_eq!(segments[2].text, format!("{}…", "后".repeat(40)));
}

const LOCAL_FIXTURE: &str = "
    INSERT INTO outbox(id, account_id, client_mutation_id, queue_canonical, message_id,
        snapshot_hash, snapshot_size, envelope, subject, recipients_preview, recipient_count,
        state, accepted_at, not_before, first_queued_at, created_at, updated_at, preview,
        server_copy_message_id, deleted_locally_at)
    VALUES
      ('00000000-0000-4000-8000-000000000001', 'account', '10000000-0000-4000-8000-000000000001', '{}', '<one@example.invalid>', 'hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh', 1, '{}', 'Local subject',
       '[\"carol@example.invalid\"]', 1, 'accepted', 't', 't', 't', 't', 't', 'local preview', NULL, NULL),
      ('00000000-0000-4000-8000-000000000002', 'account', '10000000-0000-4000-8000-000000000002', '{}', '<two@example.invalid>', 'hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh', 1, '{}', 'Matched', '[]', 1,
       'accepted', 't', 't', 't', 't', 't', NULL, 'server-id', NULL),
      ('00000000-0000-4000-8000-000000000003', 'account', '10000000-0000-4000-8000-000000000003', '{}', '<three@example.invalid>', 'hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh', 1, '{}', 'Removed', '[]', 1,
       'accepted', 't', 't', 't', 't', 't', NULL, NULL, 't');
";
