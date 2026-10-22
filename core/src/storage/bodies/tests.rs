use super::*;
use tempfile::TempDir;

fn fixture() -> (TempDir, Storage, BodyIdentity) {
    let directory = TempDir::new().unwrap();
    let store = Storage::open(directory.path()).unwrap();
    seed(&store.db);
    let identity = store.body_identity("message").unwrap();
    (directory, store, identity)
}

fn seed(db: &Connection) {
    db.execute_batch(
        "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
            VALUES ('account', 1, 'Test', 'test@example.invalid', 'source', '2026-10-19', '2026-10-19');
         INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role, selectable,
            attributes, unread_count, total_count, synced_at)
            VALUES ('folder', 'account', 'source', 'INBOX', 'Inbox', 'inbox', 1, '[]', 1, 1, '2026-10-19');
         INSERT INTO folder_sync_cursors(folder_id, uid_validity, window_start, window_capped,
            backfill_done, highest_uid, updated_at)
            VALUES ('folder', 10, '2026-09-19', 0, 1, 7, '2026-10-19');
         INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
            subject, from_address, to_addresses, cc_addresses, reply_to_addresses,
            received_at, is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
            VALUES ('message', 'account', 'source', 'folder', 10, 7, 'Synthetic body test',
            'test@example.invalid', '[]', '[]', '[]', '2026-10-19', 0, 0, '[]', 1, 'present', '2026-10-19');"
    ).unwrap();
}

fn rendered(text: &str) -> Rendered {
    Rendered {
        plain_text: Some(text.into()),
        sanitized_html: Some("<p>synthetic</p>".into()),
        links: vec!["https://example.invalid/body".into()],
        remote_images_blocked: 2,
        truncated: false,
        parts: vec![RenderedPart {
            part_index: 1,
            content_id: Some("synthetic".into()),
            filename: Some("synthetic.txt".into()),
            mime_type: "text/plain".into(),
            size: 9,
            is_inline: false,
        }],
    }
}

fn body_state(store: &Storage) -> (String, Option<String>, String) {
    store
        .db
        .query_row(
            "SELECT body_state, body_failure, preview FROM messages WHERE id = 'message'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

#[test]
fn v3_upgrade_preserves_headers_and_previous_migration_history() {
    use crate::storage::migrations::{MIGRATIONS, apply};

    let directory = TempDir::new().unwrap();
    let mut db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    apply(&mut db, &MIGRATIONS[..3]).unwrap();
    seed(&db);
    let history = |db: &Connection| {
        db.prepare(
            "SELECT version, checksum FROM schema_migrations WHERE version <= 3 ORDER BY version",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    let before = history(&db);
    drop(db);
    let store = Storage::open(directory.path()).unwrap();
    assert_eq!(history(&store.db), before);
    assert_eq!(
        store
            .db
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        MIGRATIONS.last().unwrap().version
    );
    assert_eq!(store.body_identity("message").unwrap().uid, 7);
    assert_eq!(body_state(&store), ("missing".into(), None, String::new()));
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT subject FROM messages WHERE id = 'message'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Synthetic body test"
    );
}

#[test]
fn blobs_are_hashed_reused_bounded_and_not_referenced_by_partial_writes() {
    let directory = TempDir::new().unwrap();
    let blobs = BlobStore::open(directory.path()).unwrap();
    let raw = b"Content-Type: text/plain\r\n\r\nsynthetic";
    let reference = blobs.put(raw).unwrap();
    assert_eq!(reference.sha256, format!("{:x}", Sha256::digest(raw)));
    assert_eq!(reference.size, raw.len() as u64);
    assert_eq!(blobs.read(&reference).unwrap(), raw);
    let path = blobs
        .root
        .join(&reference.sha256[..2])
        .join(&reference.sha256);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(blobs.clone().put(raw).unwrap(), reference);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);

    fs::write(&path, b"changed").unwrap();
    assert_eq!(
        blobs.read(&reference).unwrap_err().code,
        ErrorCode::StorageUnavailable
    );
    assert_eq!(
        blobs.put(raw).unwrap_err().code,
        ErrorCode::StorageUnavailable
    );
    let invalid = StoredBlob {
        sha256: "../not-a-blob".into(),
        size: 0,
    };
    assert!(blobs.read(&invalid).is_err());
    let invalid = StoredBlob {
        sha256: reference.sha256,
        size: MAX_MESSAGE_BYTES as u64 + 1,
    };
    assert!(blobs.read(&invalid).is_err());
}

#[test]
fn filesystem_error_does_not_produce_a_body_reference() {
    let (_directory, mut store, identity) = fixture();
    assert!(store.body_begin(&identity).unwrap());
    let raw = b"blocked synthetic write";
    let hash = format!("{:x}", Sha256::digest(raw));
    fs::write(store.blobs.root.join(&hash[..2]), b"not a directory").unwrap();
    assert_eq!(
        store.blobs.put(raw).unwrap_err().code,
        ErrorCode::StorageUnavailable
    );
    assert!(store.body_cache(&identity).unwrap().is_none());
    assert_eq!(body_state(&store).0, "fetching");
    assert!(
        store
            .body_fail(&identity, BodyFailureCode::Storage)
            .unwrap()
    );
    assert_eq!(body_state(&store).1.as_deref(), Some("storage"));
}

#[test]
fn concurrent_identical_puts_share_one_complete_blob() {
    let directory = TempDir::new().unwrap();
    let blobs = BlobStore::open(directory.path()).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let writers: Vec<_> = (0..4)
        .map(|_| {
            let blobs = blobs.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                blobs.put(&vec![b'x'; 64 * 1024]).unwrap()
            })
        })
        .collect();
    let references: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect();
    assert!(
        references
            .iter()
            .all(|reference| reference == &references[0])
    );
    assert_eq!(blobs.read(&references[0]).unwrap(), vec![b'x'; 64 * 1024]);
    let shard = blobs.root.join(&references[0].sha256[..2]);
    assert_eq!(fs::read_dir(shard).unwrap().count(), 1);
}

#[test]
fn commit_is_atomic_and_preview_counts_unicode_characters() {
    let (_directory, mut store, identity) = fixture();
    let blob = store.blobs.put(b"synthetic MIME").unwrap();
    let value = rendered(&"字🙂".repeat(120));
    assert!(!store.body_commit(&identity, &blob, &value).unwrap());
    assert!(store.body_begin(&identity).unwrap());
    assert!(!store.body_begin(&identity).unwrap());
    assert!(store.body_commit(&identity, &blob, &value).unwrap());
    let saved = store.body_cache(&identity).unwrap().unwrap();
    assert_eq!(saved.raw, blob);
    assert!(saved.rendered == value);
    assert_eq!(saved.sanitizer_version, SANITIZER_VERSION);
    assert_eq!(
        body_state(&store),
        ("ready".into(), None, "字🙂".repeat(100))
    );

    let mut invalid = rendered("must roll back");
    invalid.parts.push(invalid.parts[0].clone());
    assert_eq!(
        store
            .body_commit(&identity, &blob, &invalid)
            .unwrap_err()
            .code,
        ErrorCode::StorageUnavailable
    );
    assert!(store.body_cache(&identity).unwrap().unwrap().rendered == value);
    assert_eq!(body_state(&store).2, "字🙂".repeat(100));
    assert!(!store.body_reset(&identity).unwrap());
    assert!(
        !store
            .body_fail(&identity, BodyFailureCode::Protocol)
            .unwrap()
    );

    store
        .db
        .execute("UPDATE message_bodies SET sanitizer_version = 0", [])
        .unwrap();
    assert_eq!(
        store
            .body_cache(&identity)
            .unwrap()
            .unwrap()
            .sanitizer_version,
        0
    );
    assert!(
        store
            .body_commit(&identity, &blob, &rendered("rebuilt"))
            .unwrap()
    );
    assert_eq!(
        store
            .body_cache(&identity)
            .unwrap()
            .unwrap()
            .sanitizer_version,
        SANITIZER_VERSION
    );
}

#[test]
fn every_read_and_transition_rejects_a_changed_identity() {
    let mutations = [
        "UPDATE accounts SET revision = revision + 1",
        "UPDATE accounts SET mailbox_source_id = 'another-source'",
        "UPDATE folders SET selectable = 0",
        "UPDATE folders SET remote_name = 'Moved'",
        "UPDATE folder_sync_cursors SET uid_validity = 11",
        "UPDATE messages SET remote_state = 'deleted'",
        "UPDATE messages SET remote_state = 'stale'",
        "UPDATE messages SET uid = 8",
    ];
    for mutation in mutations {
        let (_directory, mut store, identity) = fixture();
        let blob = store.blobs.put(b"synthetic MIME").unwrap();
        assert!(store.body_begin(&identity).unwrap());
        store.db.execute(mutation, []).unwrap();
        assert!(
            !store.body_identity_current(&identity).unwrap(),
            "{mutation}"
        );
        assert!(!store.body_begin(&identity).unwrap(), "{mutation}");
        assert!(
            !store
                .body_commit(&identity, &blob, &rendered("old flight"))
                .unwrap(),
            "{mutation}"
        );
        assert!(
            !store
                .body_fail(&identity, BodyFailureCode::Network)
                .unwrap(),
            "{mutation}"
        );
        assert!(!store.body_reset(&identity).unwrap(), "{mutation}");
        assert!(
            matches!(
                store.body_cache(&identity),
                Err(AppError {
                    code: ErrorCode::NotFound,
                    ..
                })
            ),
            "{mutation}"
        );
        assert_eq!(body_state(&store).0, "fetching", "{mutation}");
    }
}

#[test]
fn changing_header_size_does_not_change_body_identity() {
    let (_directory, mut store, identity) = fixture();
    let blob = store.blobs.put(b"synthetic MIME").unwrap();
    assert!(store.body_begin(&identity).unwrap());
    store
        .db
        .execute("UPDATE messages SET size = 123", [])
        .unwrap();
    assert!(store.body_identity_current(&identity).unwrap());
    assert!(
        store
            .body_commit(&identity, &blob, &rendered("body"))
            .unwrap()
    );
    assert!(store.body_cache(&identity).unwrap().is_some());
}

#[test]
fn failed_writes_report_storage_instead_of_claiming_a_failed_state() {
    let (_directory, mut store, identity) = fixture();
    assert!(store.body_begin(&identity).unwrap());
    store.db.pragma_update(None, "query_only", true).unwrap();
    assert_eq!(
        store
            .body_fail(&identity, BodyFailureCode::Network)
            .unwrap_err()
            .code,
        ErrorCode::StorageUnavailable
    );
    assert_eq!(
        store.body_reset(&identity).unwrap_err().code,
        ErrorCode::StorageUnavailable
    );
    assert_eq!(body_state(&store).0, "fetching");
}

#[test]
fn resource_reads_scope_cids_and_links_to_ready_current_messages() {
    let (_directory, mut store, identity) = fixture();
    assert!(store.body_raw(&identity).unwrap().is_none());
    assert!(store.body_part(&identity, "synthetic").unwrap().is_none());
    assert!(store.body_link(&identity, 0).unwrap().is_none());
    let blob = store.blobs.put(b"synthetic CID MIME").unwrap();
    let mut value = rendered("body");
    value.parts[0].is_inline = true;
    let mut duplicate = value.parts[0].clone();
    duplicate.part_index = 3;
    value.parts.insert(0, duplicate);
    assert!(store.body_begin(&identity).unwrap());
    assert!(store.body_commit(&identity, &blob, &value).unwrap());
    assert_eq!(store.body_raw(&identity).unwrap().unwrap(), blob);
    assert_eq!(
        store.body_stamp(&identity).unwrap().unwrap(),
        (blob.clone(), SANITIZER_VERSION)
    );
    let (saved_blob, part) = store.body_part(&identity, "synthetic").unwrap().unwrap();
    assert_eq!(saved_blob, blob);
    assert_eq!(part.part_index, 1);
    assert!(store.body_part(&identity, "unknown").unwrap().is_none());
    assert_eq!(
        store.body_link(&identity, 0).unwrap().as_deref(),
        Some("https://example.invalid/body")
    );
    assert!(store.body_link(&identity, 1).unwrap().is_none());
    assert!(store.body_link(&identity, usize::MAX).unwrap().is_none());
    store
        .db
        .execute("UPDATE accounts SET revision = 2", [])
        .unwrap();
    assert_eq!(
        store.body_raw(&identity).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert!(matches!(
        store.body_part(&identity, "synthetic"),
        Err(AppError {
            code: ErrorCode::NotFound,
            ..
        })
    ));
    assert_eq!(
        store.body_link(&identity, 0).unwrap_err().code,
        ErrorCode::NotFound
    );
}

#[test]
fn sanitizer_rebuild_withdraws_ready_resources_and_can_fail_or_be_cancelled() {
    let (_directory, mut store, identity) = fixture();
    let blob = store.blobs.put(b"cached MIME").unwrap();
    assert!(!store.body_rebuild(&identity).unwrap());
    assert!(store.body_begin(&identity).unwrap());
    assert!(
        store
            .body_commit(&identity, &blob, &rendered("old"))
            .unwrap()
    );
    store
        .db
        .execute("UPDATE message_bodies SET sanitizer_version = 0", [])
        .unwrap();
    let cached = store.body_cache(&identity).unwrap().unwrap();
    assert_eq!(cached.sanitizer_version, 0);
    assert!(store.body_rebuild(&identity).unwrap());
    assert!(!store.body_rebuild(&identity).unwrap());
    assert!(store.body_cache(&identity).unwrap().is_none());
    assert!(store.body_raw(&identity).unwrap().is_none());
    assert!(
        store
            .body_fail(&identity, BodyFailureCode::Protocol)
            .unwrap()
    );
    assert_eq!(body_state(&store).1.as_deref(), Some("protocol"));
    assert!(store.body_begin(&identity).unwrap());
    assert!(
        store
            .body_commit(&identity, &cached.raw, &rendered("rebuilt"))
            .unwrap()
    );
    assert!(store.body_rebuild(&identity).unwrap());
    assert!(store.body_reset(&identity).unwrap());
    assert_eq!(body_state(&store).0, "missing");
    assert!(store.body_raw(&identity).unwrap().is_none());
}

#[test]
fn startup_recovers_fetching_and_cleans_only_unreferenced_owned_files() {
    let (directory, mut store, identity) = fixture();
    let referenced = store.blobs.put(b"retained MIME").unwrap();
    assert!(store.body_begin(&identity).unwrap());
    assert!(
        store
            .body_commit(&identity, &referenced, &rendered("cached offline"))
            .unwrap()
    );
    let orphan = store.blobs.put(b"orphan MIME").unwrap();
    let orphan_path = store
        .blobs
        .root
        .join(&orphan.sha256[..2])
        .join(&orphan.sha256);
    let shard = orphan_path.parent().unwrap();
    let temporary = shard.join(format!(".mailx-blob-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, b"unfinished").unwrap();
    let unknown = shard.join("unrelated-user-file");
    fs::write(&unknown, b"keep").unwrap();
    store
        .db
        .execute(
            "UPDATE messages SET body_state = 'fetching', body_failure = 'network'",
            [],
        )
        .unwrap();
    drop(store);
    let store = Storage::open(directory.path()).unwrap();
    assert_eq!(body_state(&store).0, "missing");
    assert!(body_state(&store).1.is_none());
    assert_eq!(store.blobs.read(&referenced).unwrap(), b"retained MIME");
    assert!(!orphan_path.exists());
    assert!(!temporary.exists());
    assert!(unknown.exists());
    assert_eq!(
        store
            .blobs
            .cleanup(&HashSet::from([referenced.sha256]))
            .unwrap(),
        0
    );
}

#[test]
fn schema_constraints_and_cascades_leave_blob_collection_for_startup() {
    let (directory, mut store, identity) = fixture();
    let blob = store.blobs.put(b"cascade MIME").unwrap();
    assert!(store.body_begin(&identity).unwrap());
    assert!(
        store
            .body_commit(&identity, &blob, &rendered("content"))
            .unwrap()
    );
    assert!(
        store
            .db
            .execute("UPDATE messages SET body_state = 'anything'", [])
            .is_err()
    );
    assert!(
        store
            .db
            .execute("UPDATE message_bodies SET truncated = 2", [])
            .is_err()
    );
    assert!(
        store
            .db
            .execute("UPDATE message_parts SET is_inline = 2", [])
            .is_err()
    );
    assert!(
        store
            .db
            .execute("UPDATE message_bodies SET raw_size = 'text'", [])
            .is_err()
    );
    store
        .db
        .execute("DELETE FROM messages WHERE id = 'message'", [])
        .unwrap();
    for table in ["message_bodies", "message_parts"] {
        assert_eq!(
            store
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
    assert!(store.blobs.read(&blob).is_ok());
    drop(store);
    let store = Storage::open(directory.path()).unwrap();
    assert!(store.blobs.read(&blob).is_err());
}

#[cfg(unix)]
#[test]
fn cleanup_and_reads_never_follow_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let blobs = BlobStore::open(directory.path()).unwrap();
    let hash = "a".repeat(64);
    let external_file = outside.path().join(&hash);
    fs::write(&external_file, b"outside").unwrap();
    symlink(outside.path(), blobs.root.join("aa")).unwrap();
    symlink(
        &external_file,
        blobs
            .root
            .join(format!(".mailx-blob-{}.tmp", Uuid::new_v4())),
    )
    .unwrap();
    assert_eq!(blobs.cleanup(&HashSet::new()).unwrap(), 0);
    assert_eq!(fs::read(&external_file).unwrap(), b"outside");
    assert!(
        blobs
            .read(&StoredBlob {
                sha256: hash,
                size: 7
            })
            .is_err()
    );

    let raw = b"symlink target";
    let hash = format!("{:x}", Sha256::digest(raw));
    let shard = blobs.root.join(&hash[..2]);
    fs::create_dir(&shard).unwrap();
    let target = outside.path().join("target");
    fs::write(&target, raw).unwrap();
    symlink(&target, shard.join(&hash)).unwrap();
    assert!(blobs.put(raw).is_err());
    assert!(
        blobs
            .read(&StoredBlob {
                sha256: hash,
                size: raw.len() as u64
            })
            .is_err()
    );
    assert_eq!(blobs.cleanup(&HashSet::new()).unwrap(), 0);
    assert_eq!(fs::read(target).unwrap(), raw);
}
