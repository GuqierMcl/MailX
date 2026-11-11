use super::*;
use tempfile::TempDir;

fn fixture() -> (TempDir, Storage) {
    let directory = TempDir::new().unwrap();
    let store = Storage::open(directory.path()).unwrap();
    store.db.execute_batch(
        "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
            VALUES ('account', 1, 'Test', 'test@example.invalid', 'source', 't', 't');
         INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
            attributes, unread_count, total_count, synced_at)
            VALUES ('folder', 'account', 'source', 'INBOX', 'Inbox', 'inbox', 1, '[]', 1, 2, 't');
         INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
            subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
            received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
            VALUES ('one', 'account', 'source', 'folder', 10, 1, 'Hello',
            'a@example.invalid', '[]', '[]', '[]', 't', 0, 0, '[]', 0, 'present', 't'),
            ('two', 'account', 'source', 'folder', 10, 2, 'Again',
            'a@example.invalid', '[]', '[]', '[]', 't', 0, 0, '[]', 0, 'present', 't');",
    )
    .unwrap();
    (directory, store)
}

fn segments() -> Vec<Option<String>> {
    vec![
        Some("你好".into()),
        None,
        Some("<span id=\"1\">好</span>".into()),
    ]
}

fn row<'a>(
    message: &'a str,
    target: &'a str,
    quotes: bool,
    segments: &'a [Option<String>],
) -> NewTranslation<'a> {
    NewTranslation {
        message_id: message,
        target_language: target,
        include_quotes: quotes,
        raw_sha256: "raw",
        sanitizer_version: 3,
        segmenter_version: 1,
        provider: TranslationProviderId::Deepl,
        source_language: Some("en"),
        subject: Some("你好"),
        segments,
        partial: true,
    }
}

fn key<'a>(message: &'a str, target: &'a str, quotes: bool) -> TranslationKey<'a> {
    TranslationKey {
        message_id: message,
        target_language: target,
        include_quotes: quotes,
        raw_sha256: "raw",
        sanitizer_version: 3,
        segmenter_version: 1,
    }
}

#[test]
fn a_row_round_trips_and_is_keyed_by_target_and_quotes() {
    let (_directory, mut store) = fixture();
    let segments = segments();
    store
        .translation_put(&row("one", "zh-Hans", false, &segments))
        .unwrap();
    let stored = store
        .translation_get(&key("one", "zh-Hans", false))
        .unwrap()
        .unwrap();
    assert_eq!(
        stored,
        StoredTranslation {
            provider: TranslationProviderId::Deepl,
            source_language: Some("en".into()),
            subject: Some("你好".into()),
            segments,
            partial: true,
        }
    );
    assert!(
        store
            .translation_get(&key("one", "zh-Hant", false))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .translation_get(&key("one", "zh-Hans", true))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .translation_get(&key("two", "zh-Hans", false))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_second_write_replaces_the_row() {
    let (_directory, mut store) = fixture();
    let first = segments();
    let second = vec![Some("new".to_owned())];
    store
        .translation_put(&row("one", "en", false, &first))
        .unwrap();
    store
        .translation_put(&row("one", "en", false, &second))
        .unwrap();
    let stored = store
        .translation_get(&key("one", "en", false))
        .unwrap()
        .unwrap();
    assert_eq!(stored.segments, second);
    assert_eq!(store.translation_stats().unwrap().count, 1);
}

#[test]
fn a_changed_body_or_version_makes_the_row_stale() {
    let (_directory, mut store) = fixture();
    let segments = segments();
    store
        .translation_put(&row("one", "en", false, &segments))
        .unwrap();
    for stale in [
        TranslationKey {
            raw_sha256: "other",
            ..key("one", "en", false)
        },
        TranslationKey {
            sanitizer_version: 4,
            ..key("one", "en", false)
        },
        TranslationKey {
            segmenter_version: 2,
            ..key("one", "en", false)
        },
    ] {
        assert!(store.translation_get(&stale).unwrap().is_none());
    }
    assert!(
        store
            .translation_get(&key("one", "en", false))
            .unwrap()
            .is_some()
    );
}

#[test]
fn delete_one_message_or_everything() {
    let (_directory, mut store) = fixture();
    let segments = segments();
    for (message, target) in [("one", "en"), ("one", "de"), ("two", "en")] {
        store
            .translation_put(&row(message, target, false, &segments))
            .unwrap();
    }
    store.translation_delete_message("one").unwrap();
    assert_eq!(store.translation_stats().unwrap().count, 1);
    assert!(
        store
            .translation_get(&key("two", "en", false))
            .unwrap()
            .is_some()
    );
    store.translation_delete_all().unwrap();
    assert_eq!(
        store.translation_stats().unwrap(),
        TranslationStats { count: 0, bytes: 0 }
    );
}

#[test]
fn stats_count_rows_and_text_bytes() {
    let (_directory, mut store) = fixture();
    let segments = vec![Some("abc".to_owned())];
    let mut value = row("one", "en", false, &segments);
    value.subject = Some("你好");
    store.translation_put(&value).unwrap();
    let stats = store.translation_stats().unwrap();
    // `["abc"]` is 7 bytes; the subject is two 3-byte characters.
    assert_eq!(
        stats,
        TranslationStats {
            count: 1,
            bytes: 13
        }
    );
}

#[test]
fn removing_a_message_folder_or_account_removes_its_translations() {
    let segments = segments();
    for statement in [
        "DELETE FROM messages WHERE id = 'one'",
        "DELETE FROM folders WHERE id = 'folder'",
        "DELETE FROM accounts WHERE id = 'account'",
    ] {
        let (_directory, mut store) = fixture();
        store
            .translation_put(&row("one", "en", false, &segments))
            .unwrap();
        store
            .translation_put(&row("two", "en", false, &segments))
            .unwrap();
        store.db.execute(statement, []).unwrap();
        let left = store.translation_stats().unwrap().count;
        assert_eq!(left, u64::from(statement.contains("'one'")), "{statement}");
    }
}

#[test]
fn local_sent_copies_are_not_message_rows() {
    // Their ids have no `messages` row, so the foreign key refuses a cache
    // entry; the service never translates them.
    let (_directory, mut store) = fixture();
    let segments = segments();
    assert!(
        store
            .translation_put(&row("local-sent:abc", "en", false, &segments))
            .is_err()
    );
}

#[test]
fn the_subject_comes_from_the_message_row() {
    let (_directory, store) = fixture();
    assert_eq!(
        store.translation_subject("one").unwrap().as_deref(),
        Some("Hello")
    );
    assert_eq!(store.translation_subject("none").unwrap(), None);
}
