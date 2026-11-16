use super::*;
use tempfile::TempDir;

fn fixture() -> (TempDir, Storage, SyncTarget) {
    let directory = TempDir::new().unwrap();
    let storage = Storage::open(directory.path()).unwrap();
    storage.db.execute_batch(
        "INSERT INTO accounts(id,revision,display_name,email_address,mailbox_source_id,created_at,updated_at)
         VALUES ('account',1,'Synthetic','synthetic@example.invalid','source','2026-11-16','2026-11-16');
         INSERT INTO folders(id,account_id,mailbox_source_id,remote_name,name,role,selectable,attributes,unread_count,total_count,synced_at)
         VALUES ('folder','account','source','remote-folder','Inbox','inbox',1,'[]',0,0,'');
         INSERT INTO folder_sync_cursors(folder_id,uid_validity,window_start,window_capped,backfill_done,highest_uid,updated_at)
         VALUES ('folder',1,'2026-10-16',0,0,0,'2026-11-16');"
    ).unwrap();
    let target = SyncTarget {
        account_id: "account".into(),
        mailbox_source_id: "source".into(),
        folder_id: "folder".into(),
        uid_validity: UID_VALIDITY,
    };
    (directory, storage, target)
}

fn keys(values: &[(&str, &str)]) -> Vec<(String, Option<String>)> {
    values
        .iter()
        .map(|(id, key)| ((*id).into(), Some((*key).into())))
        .collect()
}

#[test]
fn ews_identifiers_are_stable_across_order_key_changes_duplicates_and_restart() {
    let (directory, mut storage, target) = fixture();
    let assigned = storage
        .ews_assign(
            &target,
            &keys(&[
                ("opaque-a", "key1"),
                ("opaque-b", "key1"),
                ("opaque-a", "key2"),
            ]),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        assigned.iter().map(|item| item.uid).collect::<Vec<_>>(),
        [1, 2, 1]
    );
    drop(storage);
    let mut storage = Storage::open(directory.path()).unwrap();
    let assigned = storage
        .ews_assign(
            &target,
            &keys(&[
                ("opaque-b", "key2"),
                ("opaque-a", "key3"),
                ("opaque-c", "key1"),
            ]),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        assigned.iter().map(|item| item.uid).collect::<Vec<_>>(),
        [2, 1, 3]
    );
    assert_eq!(
        storage
            .ews_item(&target, 1)
            .unwrap()
            .unwrap()
            .change_key
            .as_deref(),
        Some("key3")
    );
}

#[test]
fn ews_mapping_rejects_stale_sources_accounts_and_generations() {
    let (_directory, mut storage, target) = fixture();
    storage
        .ews_assign(&target, &keys(&[("opaque", "key")]))
        .unwrap()
        .unwrap();
    for stale in [
        SyncTarget {
            account_id: "other-account".into(),
            ..target.clone()
        },
        SyncTarget {
            mailbox_source_id: "old-source".into(),
            ..target.clone()
        },
        SyncTarget {
            uid_validity: 2,
            ..target.clone()
        },
    ] {
        assert!(
            storage
                .ews_assign(&stale, &keys(&[("new", "key")]))
                .unwrap()
                .is_none()
        );
        assert!(storage.ews_item(&stale, 1).unwrap().is_none());
    }
    storage
        .db
        .execute("UPDATE accounts SET mailbox_source_id='new-source'", [])
        .unwrap();
    assert!(storage.ews_item(&target, 1).unwrap().is_none());
    assert!(
        storage
            .ews_assign(&target, &keys(&[("new", "key")]))
            .unwrap()
            .is_none()
    );
}

#[test]
fn ews_mapping_never_reuses_uids_and_folder_removal_cascades_it() {
    let (_directory, mut storage, target) = fixture();
    storage
        .ews_assign(&target, &keys(&[("old", "key")]))
        .unwrap()
        .unwrap();
    // A missing remote item is retained in this map, so an old body or queued
    // action can never resolve its uid to the next unrelated message.
    let next = storage
        .ews_assign(&target, &keys(&[("new", "key")]))
        .unwrap()
        .unwrap();
    assert_eq!(next[0].uid, 2);
    assert_eq!(
        storage.ews_item(&target, 1).unwrap().unwrap().item_id,
        "old"
    );
    storage
        .db
        .execute("DELETE FROM folders WHERE id='folder'", [])
        .unwrap();
    assert_eq!(
        storage
            .db
            .query_row("SELECT COUNT(*) FROM ews_items", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        storage
            .db
            .query_row("SELECT COUNT(*) FROM ews_folder_state", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn ews_assign_writes_only_new_items_and_changed_keys() {
    let (_directory, mut storage, target) = fixture();
    storage
        .ews_assign(&target, &keys(&[("a", "key1"), ("b", "key1")]))
        .unwrap()
        .unwrap();
    storage
        .db
        .execute_batch(
            "CREATE TEMP TABLE writes(kind TEXT);
             CREATE TEMP TRIGGER item_update AFTER UPDATE ON ews_items
             BEGIN INSERT INTO writes VALUES ('item'); END;
             CREATE TEMP TRIGGER state_update AFTER UPDATE ON ews_folder_state
             BEGIN INSERT INTO writes VALUES ('state'); END;",
        )
        .unwrap();
    let writes = |storage: &Storage| -> Vec<String> {
        storage
            .db
            .prepare("SELECT kind FROM writes ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let quiet = storage
        .ews_assign(&target, &keys(&[("a", "key1"), ("b", "key1")]))
        .unwrap()
        .unwrap();
    assert_eq!(
        quiet.iter().map(|item| item.uid).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(
        writes(&storage).is_empty(),
        "an unchanged listing writes nothing"
    );
    storage
        .ews_assign(&target, &keys(&[("a", "key1"), ("b", "key2")]))
        .unwrap()
        .unwrap();
    assert_eq!(writes(&storage), ["item"]);
    assert_eq!(
        storage
            .ews_item(&target, 2)
            .unwrap()
            .unwrap()
            .change_key
            .as_deref(),
        Some("key2")
    );
}
