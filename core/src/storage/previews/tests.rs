use super::*;
use rusqlite::Connection;
use tempfile::TempDir;

fn fixture() -> (TempDir, Storage) {
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
    for uid in 1..=3 {
        add(&store.db, &format!("m{uid}"), uid);
    }
    (directory, store)
}

fn add(db: &Connection, id: &str, uid: u32) {
    db.execute(
        "INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
            subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
            received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
         VALUES (?1, 'account', 'source', 'folder', 10, ?2, 'Synthetic', 'test@example.invalid',
            '[]', '[]', '[]', '2026-10-19', 0, 0, '[]', 0, 'present', '2026-10-19')",
        rusqlite::params![id, uid],
    )
    .unwrap();
}

fn state(store: &Storage, id: &str) -> (String, String) {
    store
        .db
        .query_row(
            "SELECT preview_state, preview FROM messages WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn targets_skip_resolved_and_unknown_rows() {
    let (_directory, mut store) = fixture();
    let all = ids(&["m1", "m2", "m3", "unknown"]);
    assert_eq!(store.preview_targets(&all).unwrap().len(), 3);
    let target = store.preview_targets(&ids(&["m1"])).unwrap().remove(0);
    store
        .preview_commit(&[PreviewOutcome {
            identity: target,
            preview: Some("hello".into()),
        }])
        .unwrap();
    store
        .db
        .execute(
            "UPDATE messages SET preview_state = 'failed' WHERE id = 'm2'",
            [],
        )
        .unwrap();
    let remaining = store.preview_targets(&all).unwrap();
    assert_eq!(
        remaining
            .iter()
            .map(|t| t.message_id.as_str())
            .collect::<Vec<_>>(),
        ["m3"]
    );
    assert_eq!((remaining[0].uid, remaining[0].uid_validity), (3, 10));
    assert_eq!(remaining[0].remote_name, "INBOX");
}

#[test]
fn commit_marks_ready_or_failed_and_reports_each() {
    let (_directory, mut store) = fixture();
    let mut targets = store.preview_targets(&ids(&["m1", "m2"])).unwrap();
    let second = targets.pop().unwrap();
    let first = targets.pop().unwrap();
    let written = store
        .preview_commit(&[
            PreviewOutcome {
                identity: first,
                preview: Some("first line".into()),
            },
            PreviewOutcome {
                identity: second,
                preview: None,
            },
        ])
        .unwrap();
    assert_eq!(
        written,
        [
            PreviewWritten {
                message_id: "m1".into(),
                preview: "first line".into(),
                state: PreviewState::Ready
            },
            PreviewWritten {
                message_id: "m2".into(),
                preview: String::new(),
                state: PreviewState::Failed
            },
        ]
    );
    assert_eq!(state(&store, "m1"), ("ready".into(), "first line".into()));
    assert_eq!(state(&store, "m2"), ("failed".into(), String::new()));
}

#[test]
fn partial_preview_never_overwrites_a_full_body_preview() {
    let (_directory, mut store) = fixture();
    let target = store.preview_targets(&ids(&["m1"])).unwrap().remove(0);
    store
        .db
        .execute(
            "UPDATE messages SET body_state = 'ready', preview_state = 'ready', preview = 'full body'
             WHERE id = 'm1'",
            [],
        )
        .unwrap();
    let written = store
        .preview_commit(&[PreviewOutcome {
            identity: target,
            preview: Some("partial".into()),
        }])
        .unwrap();
    assert!(written.is_empty());
    assert_eq!(state(&store, "m1"), ("ready".into(), "full body".into()));
    assert!(store.preview_targets(&ids(&["m1"])).unwrap().is_empty());
}

#[test]
fn stale_identity_is_skipped() {
    let (_directory, mut store) = fixture();
    let target = store.preview_targets(&ids(&["m1"])).unwrap().remove(0);
    store
        .db
        .execute("UPDATE folder_sync_cursors SET uid_validity = 11", [])
        .unwrap();
    assert!(store.preview_targets(&ids(&["m1"])).unwrap().is_empty());
    let written = store
        .preview_commit(&[PreviewOutcome {
            identity: target,
            preview: Some("late".into()),
        }])
        .unwrap();
    assert!(written.is_empty());
    assert_eq!(state(&store, "m1").0, "missing");
}

#[test]
fn constraint_rejects_unknown_states() {
    let (_directory, store) = fixture();
    assert!(
        store
            .db
            .execute(
                "UPDATE messages SET preview_state = 'bogus' WHERE id = 'm1'",
                []
            )
            .is_err()
    );
}
