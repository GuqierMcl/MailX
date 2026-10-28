use super::*;
use crate::accounts::sync_plan::tests::{config, connection_update};
use crate::accounts::types::{Account, CreateAccountRequest};
use crate::secrets::{SecretStore, SecretStoreError, SecretValue};
use crate::send::types::{OutboxItem, QueueSendRequest};
use crate::storage::bodies::StoredBlob;
use crate::storage::encode;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Barrier, Mutex};
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Default)]
struct Secrets(Mutex<SecretState>);
#[derive(Default)]
struct SecretState {
    entries: HashMap<String, SecretValue>,
    locked: bool,
    fail_set_after_write: bool,
    fail_delete_after_write: bool,
    reads: usize,
}
impl SecretStore for Secrets {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        let mut state = self.0.lock().unwrap();
        state.reads += 1;
        if state.locked {
            return Err(SecretStoreError::Locked);
        }
        Ok(state.entries.get(key).cloned())
    }
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        let mut state = self.0.lock().unwrap();
        if state.locked {
            return Err(SecretStoreError::Locked);
        }
        state.entries.insert(key.into(), value.clone());
        if state.fail_set_after_write {
            return Err(SecretStoreError::Unavailable);
        }
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        let mut state = self.0.lock().unwrap();
        if state.locked {
            return Err(SecretStoreError::Locked);
        }
        state.entries.remove(key);
        if state.fail_delete_after_write {
            return Err(SecretStoreError::Unavailable);
        }
        Ok(())
    }
}

struct Fixture {
    directory: TempDir,
    secrets: Arc<Secrets>,
    service: AccountService,
    account: Account,
    create_id: String,
}
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn request(account: &Account) -> RemoveAccountRequest {
    RemoveAccountRequest {
        account_id: account.id.clone(),
        expected_revision: account.revision,
    }
}
fn create_request(request_id: &str, value: Value) -> CreateAccountRequest {
    serde_json::from_value(json!({"requestId": request_id, "config": value})).unwrap()
}
fn setup() -> Fixture {
    let directory = TempDir::new().unwrap();
    let secrets = Arc::new(Secrets::default());
    let mut service = AccountService::open(directory.path(), secrets.clone()).unwrap();
    let create_id = id();
    let account = service
        .create(create_request(&create_id, config(json!({"kind":"direct"}))))
        .unwrap();
    Fixture {
        directory,
        secrets,
        service,
        account,
        create_id,
    }
}
fn count(service: &AccountService, table: &str) -> u32 {
    service
        .storage
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
fn queue(service: &mut AccountService, account: &str) -> (String, OutboxItem) {
    let draft = service
        .create_draft(
            serde_json::from_value(json!({
                "accountId": account, "to": [{"name":null,"address":"recipient@example.test"}],
                "cc": [], "bcc": [], "subject":"Synthetic", "bodyHtml":"<p>Local only</p>",
                "clientMutationId": id(),
            }))
            .unwrap(),
            "",
        )
        .unwrap();
    let (item, _) = service
        .queue_send(
            &QueueSendRequest {
                draft_id: draft.id.clone(),
                expected_revision: draft.revision,
                client_mutation_id: id(),
            },
            0,
            chrono::Utc::now(),
        )
        .unwrap();
    (draft.id, item)
}
fn seed_message(
    service: &AccountService,
    account: &Account,
    source: &str,
    raw: &[u8],
) -> StoredBlob {
    let blob = service.storage.blobs.put(raw).unwrap();
    let folder = id();
    let message = id();
    let db = &service.storage.db;
    db.execute(
        "INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, name, role,
         selectable, attributes, unread_count, total_count, synced_at)
         VALUES (?1, ?2, ?3, 'INBOX', 'Inbox', 'inbox', 1, '[]', 1, 1, '2026-10-28')",
        params![folder, account.id, source],
    )
    .unwrap();
    db.execute(
        "INSERT INTO folder_sync_cursors(folder_id, uid_validity, window_start, window_capped,
         backfill_done, highest_uid, updated_at) VALUES (?1, 1, '2026-10-01', 0, 1, 1, '2026-10-28')",
        [&folder],
    )
    .unwrap();
    db.execute(
        "INSERT INTO messages(id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
         subject, from_address, to_addresses, cc_addresses, reply_to_addresses, received_at,
         is_read, is_flagged, flags, has_attachment, remote_state, updated_at)
         VALUES (?1, ?2, ?3, ?4, 1, 1, 'Local', 'sender@example.test', '[]', '[]', '[]',
         '2026-10-28', 0, 0, '[]', 1, 'present', '2026-10-28')",
        params![message, account.id, source, folder],
    )
    .unwrap();
    db.execute(
        "INSERT INTO message_bodies(message_id, raw_sha256, raw_size, sanitizer_version, links,
         remote_images_blocked, truncated, fetched_at) VALUES (?1, ?2, ?3, 1, '[]', 0, 0, '2026-10-28')",
        params![message, blob.sha256, blob.size as i64],
    )
    .unwrap();
    db.execute(
        "INSERT INTO message_parts(message_id, part_index, mime_type, size, is_inline)
         VALUES (?1, 0, 'text/plain', 4, 0)",
        [&message],
    )
    .unwrap();
    db.execute(
        "INSERT INTO flag_changes(message_id, flag, wanted, created_at, updated_at)
         VALUES (?1, 'seen', 1, '2026-10-28', '2026-10-28')",
        [&message],
    )
    .unwrap();
    blob
}

#[test]
fn removal_cascades_every_source_and_queue_state_without_touching_another_account() {
    let mut f = setup();
    let mut other_config = config(json!({"kind":"direct"}));
    other_config["emailAddress"] = json!("other@example.test");
    let other = f
        .service
        .create(create_request(&id(), other_config))
        .unwrap();
    let orphan = seed_message(&f.service, &f.account, "old-source", b"old local body");
    let shared = seed_message(
        &f.service,
        &f.account,
        &f.account.mailbox_source_id,
        b"shared local body",
    );
    seed_message(
        &f.service,
        &other,
        &other.mailbox_source_id,
        b"shared local body",
    );
    let mut drafts = Vec::new();
    for state in ["queued", "unknown", "accepted", "failed", "canceled"] {
        let (draft, item) = queue(&mut f.service, &f.account.id);
        drafts.push(draft);
        f.service
            .storage
            .db
            .execute(
                "UPDATE outbox SET state = ?2,
             accepted_at = CASE WHEN ?2 = 'accepted' THEN '2026-10-28' END WHERE id = ?1",
                params![item.id, state],
            )
            .unwrap();
    }
    let (other_draft, other_item) = queue(&mut f.service, &other.id);
    let removed = f.service.remove(request(&f.account)).unwrap();
    drafts.sort();
    assert_eq!(removed.draft_ids, drafts);
    assert!(!removed.credentials_pending);
    for table in [
        "accounts",
        "auth_profiles",
        "network_routes",
        "create_requests",
        "folders",
        "folder_sync_cursors",
        "messages",
        "message_bodies",
        "message_parts",
        "flag_changes",
        "drafts",
        "outbox",
    ] {
        assert_eq!(count(&f.service, table), 1, "{table}");
    }
    assert_eq!(count(&f.service, "mail_connections"), 2);
    assert_eq!(f.service.get(&other.id).unwrap(), other);
    assert!(f.service.get_draft(&other_draft).is_ok());
    assert!(f.service.get_outbox(&other_item.id).is_ok());
    assert!(
        f.service
            .storage
            .db
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    assert!(
        f.service.storage.blobs.read(&orphan).is_ok(),
        "runtime must not GC blobs"
    );
    drop(f.service);
    let reopened = AccountService::open(f.directory.path(), f.secrets).unwrap();
    assert!(reopened.storage.blobs.read(&orphan).is_err());
    assert!(
        reopened.storage.blobs.read(&shared).is_ok(),
        "another account still references it"
    );
}

#[test]
fn sending_stale_missing_and_invalid_requests_change_nothing() {
    let mut f = setup();
    let (_, item) = queue(&mut f.service, &f.account.id);
    f.service
        .force_sending_for_tests(&[(&item.id, true)], chrono::Utc::now())
        .unwrap();
    let keys = f.secrets.0.lock().unwrap().entries.len();
    assert_eq!(
        f.service.remove(request(&f.account)).unwrap_err().code,
        ErrorCode::AlreadySending
    );
    let mut stale = request(&f.account);
    stale.expected_revision += 1;
    assert_eq!(
        f.service.remove(stale).unwrap_err().code,
        ErrorCode::Conflict
    );
    let mut missing = request(&f.account);
    missing.account_id = id();
    assert_eq!(
        f.service.remove(missing).unwrap_err().code,
        ErrorCode::NotFound
    );
    let mut invalid = request(&f.account);
    invalid.expected_revision = 0;
    assert_eq!(
        f.service.remove(invalid).unwrap_err().code,
        ErrorCode::Validation
    );
    assert_eq!(count(&f.service, "account_removals"), 0);
    assert_eq!(count(&f.service, "secret_gc"), 0);
    assert_eq!(count(&f.service, "removed_create_requests"), 0);
    assert_eq!(count(&f.service, "outbox"), 1);
    assert_eq!(count(&f.service, "drafts"), 1);
    assert_eq!(f.secrets.0.lock().unwrap().entries.len(), keys);
    assert_eq!(f.service.get(&f.account.id).unwrap(), f.account);
}

#[test]
fn failed_sql_rolls_back_data_receipt_tombstones_and_cleanup_intent() {
    let mut f = setup();
    queue(&mut f.service, &f.account.id);
    f.service
        .storage
        .db
        .execute_batch(
            "CREATE TEMP TRIGGER reject_removal BEFORE DELETE ON accounts
         BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;",
        )
        .unwrap();
    assert!(f.service.remove(request(&f.account)).is_err());
    for table in [
        "accounts",
        "drafts",
        "outbox",
        "create_requests",
        "auth_profiles",
    ] {
        assert_eq!(count(&f.service, table), 1, "{table}");
    }
    for table in ["account_removals", "removed_create_requests", "secret_gc"] {
        assert_eq!(count(&f.service, table), 0, "{table}");
    }
    assert_eq!(f.secrets.0.lock().unwrap().entries.len(), 1);
}

#[test]
fn removal_is_retryable_after_restart_and_old_create_requests_stay_consumed() {
    let mut f = setup();
    queue(&mut f.service, &f.account.id);
    let removed = f.service.remove(request(&f.account)).unwrap();
    assert_eq!(f.service.remove(request(&f.account)).unwrap(), removed);
    drop(f.service);
    let mut reopened = AccountService::open(f.directory.path(), f.secrets.clone()).unwrap();
    assert_eq!(reopened.remove(request(&f.account)).unwrap(), removed);
    f.secrets.0.lock().unwrap().locked = true;
    // Tombstones are checked before any host credential lookup/recovery.
    assert_eq!(
        reopened
            .create(create_request(
                &f.create_id,
                config(json!({"kind":"direct"}))
            ))
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    );
    assert_eq!(count(&reopened, "accounts"), 0);
    let mut stale = request(&f.account);
    stale.expected_revision += 1;
    assert_eq!(
        reopened.remove(stale).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[test]
fn a_locked_store_does_not_block_removal_or_reopening_and_is_retried_on_a_write() {
    let mut f = setup();
    let other = f
        .service
        .create(create_request(&id(), config(json!({"kind":"direct"}))))
        .unwrap();
    let before_reads = f.secrets.0.lock().unwrap().reads;
    f.secrets.0.lock().unwrap().locked = true;
    let removed = f.service.remove(request(&f.account)).unwrap();
    assert!(removed.credentials_pending);
    assert_eq!(f.secrets.0.lock().unwrap().reads, before_reads);
    assert_eq!(count(&f.service, "secret_gc"), 1);
    drop(f.service);
    let mut reopened = AccountService::open(f.directory.path(), f.secrets.clone()).unwrap();
    assert!(
        reopened
            .remove(request(&f.account))
            .unwrap()
            .credentials_pending
    );
    assert_eq!(count(&reopened, "accounts"), 1);
    f.secrets.0.lock().unwrap().locked = false;
    reopened
        .update_profile(
            serde_json::from_value(json!({
                "accountId": other.id, "profile":{"displayName":"Kept", "color":null},
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(count(&reopened, "secret_gc"), 0);
    assert_eq!(f.secrets.0.lock().unwrap().entries.len(), 1);
    assert!(
        !reopened
            .remove(request(&f.account))
            .unwrap()
            .credentials_pending
    );
}

#[test]
fn crashing_after_the_host_deleted_a_key_retries_the_acknowledgement() {
    let mut f = setup();
    f.secrets.0.lock().unwrap().fail_delete_after_write = true;
    assert!(
        f.service
            .remove(request(&f.account))
            .unwrap()
            .credentials_pending
    );
    assert!(f.secrets.0.lock().unwrap().entries.is_empty());
    assert_eq!(count(&f.service, "secret_gc"), 1);
    drop(f.service);
    f.secrets.0.lock().unwrap().fail_delete_after_write = false;
    let mut reopened = AccountService::open(f.directory.path(), f.secrets).unwrap();
    assert!(
        !reopened
            .remove(request(&f.account))
            .unwrap()
            .credentials_pending
    );
    assert_eq!(count(&reopened, "secret_gc"), 0);
    assert_eq!(count(&reopened, "accounts"), 0);
}

#[test]
fn removal_collects_applied_and_unapplied_journals_without_applying_them() {
    let mut f = setup();
    let mut replacement = config(
        json!({"kind":"custom", "proxyKind":"socks5", "host":"proxy.example.test",
        "port":1080,"dns":"proxy","authentication":{"username":"proxy", "password":{"action":"replace","value":"proxy-secret"}}}),
    );
    replacement["authentication"]["password"] = json!({"action":"replace","value":"replacement"});
    // Keep a committed journal's retired key after the connection update.
    f.secrets.0.lock().unwrap().fail_delete_after_write = true;
    f.account = f
        .service
        .update_connection(connection_update(
            &f.account.id,
            f.account.revision,
            replacement,
        ))
        .unwrap();
    assert_eq!(count(&f.service, "credential_journal"), 1);
    let mut next = config(json!({"kind":"direct"}));
    next["authentication"]["password"] = json!({"action":"replace","value":"uncommitted"});
    // A fresh key exists, but the connection snapshot was never applied.
    f.secrets.0.lock().unwrap().fail_set_after_write = true;
    assert!(
        f.service
            .update_connection(connection_update(&f.account.id, f.account.revision, next))
            .is_err()
    );
    assert_eq!(count(&f.service, "credential_journal"), 2);
    // Both journal shapes may have been written before provider icon preferences.
    let legacy_snapshots: Vec<(String, String)> = f
        .service
        .storage
        .db
        .prepare("SELECT id, snapshot_json FROM credential_journal")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (journal_id, snapshot) in legacy_snapshots {
        let mut snapshot: Value = serde_json::from_str(&snapshot).unwrap();
        snapshot["account"]
            .as_object_mut()
            .unwrap()
            .remove("useProviderIcon");
        f.service
            .storage
            .db
            .execute(
                "UPDATE credential_journal SET snapshot_json=?2 WHERE id=?1",
                rusqlite::params![journal_id, serde_json::to_string(&snapshot).unwrap()],
            )
            .unwrap();
    }
    f.secrets.0.lock().unwrap().fail_delete_after_write = false;
    f.secrets.0.lock().unwrap().locked = true;
    let removed = f.service.remove(request(&f.account)).unwrap();
    assert!(removed.credentials_pending);
    assert_eq!(count(&f.service, "credential_journal"), 0);
    assert!(count(&f.service, "secret_gc") >= 3);
    drop(f.service);
    f.secrets.0.lock().unwrap().locked = false;
    f.secrets.0.lock().unwrap().fail_set_after_write = false;
    let reopened = AccountService::open(f.directory.path(), f.secrets.clone()).unwrap();
    assert_eq!(count(&reopened, "accounts"), 0);
    assert_eq!(count(&reopened, "credential_journal"), 0);
    assert_eq!(count(&reopened, "secret_gc"), 0);
    assert!(f.secrets.0.lock().unwrap().entries.is_empty());
}

#[test]
fn an_unapplied_creation_journal_is_tombstoned_and_cannot_restore_the_account() {
    let mut f = setup();
    let mut journal = f.service.storage.load(&f.account.id).unwrap();
    journal.account.revision = 1;
    let delayed_create_id = id();
    let key = "synthetic-pending-creation-key".to_owned();
    f.secrets
        .set(&key, &SecretValue::new("synthetic".into()))
        .unwrap();
    f.service
        .storage
        .prepare(&crate::accounts::model::Journal {
            id: id(),
            expected_revision: None,
            snapshot: journal,
            new_refs: vec![key.clone()],
            retired_refs: vec!["synthetic-retired-key".into()],
            create_request: Some((delayed_create_id.clone(), "synthetic".into())),
            applied: false,
        })
        .unwrap();
    f.service.remove(request(&f.account)).unwrap();
    assert_eq!(count(&f.service, "credential_journal"), 0);
    assert!(
        f.service
            .storage
            .create_request_was_removed(&delayed_create_id)
            .unwrap()
    );
    drop(f.service);
    let mut reopened = AccountService::open(f.directory.path(), f.secrets).unwrap();
    assert_eq!(
        reopened
            .create(create_request(
                &delayed_create_id,
                config(json!({"kind":"direct"}))
            ))
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    );
    assert_eq!(count(&reopened, "accounts"), 0);
}

#[test]
fn a_crash_immediately_after_the_database_commit_keeps_cleanup_intent() {
    let mut f = setup();
    let removed_drafts = f
        .service
        .storage
        .remove_account(&request(&f.account))
        .unwrap();
    assert!(removed_drafts.is_empty());
    assert_eq!(f.secrets.0.lock().unwrap().entries.len(), 1);
    assert_eq!(count(&f.service, "secret_gc"), 1);
    drop(f.service);
    let reopened = AccountService::open(f.directory.path(), f.secrets.clone()).unwrap();
    assert_eq!(count(&reopened, "accounts"), 0);
    assert_eq!(count(&reopened, "secret_gc"), 0);
    assert!(f.secrets.0.lock().unwrap().entries.is_empty());
}

#[test]
fn a_shared_historical_key_never_deletes_another_accounts_live_credential() {
    let mut f = setup();
    let other = f
        .service
        .create(create_request(&id(), config(json!({"kind":"direct"}))))
        .unwrap();
    let other_snapshot = f.service.storage.load(&other.id).unwrap();
    let snapshot = f.service.storage.load(&f.account.id).unwrap();
    f.service
        .storage
        .prepare(&crate::accounts::model::Journal {
            id: id(),
            expected_revision: Some(f.account.revision),
            snapshot,
            new_refs: vec![],
            retired_refs: other_snapshot.refs(),
            create_request: None,
            applied: true,
        })
        .unwrap();
    // Simulates an old journal that accidentally mentions another live key.
    let removed = f.service.remove(request(&f.account)).unwrap();
    assert!(removed.credentials_pending);
    assert!(
        f.service
            .get(&other.id)
            .unwrap()
            .authentication
            .imap
            .password_configured
    );
    assert_eq!(f.secrets.0.lock().unwrap().entries.len(), 1);
}

#[test]
fn concurrent_queue_claim_and_removal_have_one_serialized_winner() {
    for _ in 0..8 {
        let mut f = setup();
        queue(&mut f.service, &f.account.id);
        let service = Arc::new(Mutex::new(f.service));
        let barrier = Arc::new(Barrier::new(3));
        let claimed_service = service.clone();
        let claimed_barrier = barrier.clone();
        let claimed_id = f.account.id.clone();
        let claim = std::thread::spawn(move || {
            claimed_barrier.wait();
            let step = claimed_service
                .lock()
                .unwrap()
                .outbox_step(&claimed_id, chrono::Utc::now())
                .unwrap();
            matches!(step.next, crate::accounts::Next::Job { .. })
        });
        let removed_service = service.clone();
        let removed_barrier = barrier.clone();
        let removal = request(&f.account);
        let remove = std::thread::spawn(move || {
            removed_barrier.wait();
            removed_service.lock().unwrap().remove(removal)
        });
        barrier.wait();
        let claimed = claim.join().unwrap();
        let removed = remove.join().unwrap();
        if claimed {
            assert_eq!(removed.unwrap_err().code, ErrorCode::AlreadySending);
            assert_eq!(count(&service.lock().unwrap(), "accounts"), 1);
        } else {
            removed.unwrap();
            assert_eq!(count(&service.lock().unwrap(), "accounts"), 0);
        }
    }
}

#[test]
fn malformed_journal_never_partially_deletes_an_account() {
    let mut f = setup();
    let snapshot = f.service.storage.load(&f.account.id).unwrap();
    f.service.storage.db.execute(
        "INSERT INTO credential_journal(id, account_id, expected_revision, snapshot_json,
         new_refs_json, retired_refs_json, applied) VALUES (?1, ?2, 1, ?3, 'invalid-json', '[]', 0)",
        params![id(), f.account.id, encode(&snapshot).unwrap()],
    ).unwrap();
    assert_eq!(
        f.service.remove(request(&f.account)).unwrap_err().code,
        ErrorCode::StorageIncompatible
    );
    assert_eq!(count(&f.service, "accounts"), 1);
    assert_eq!(count(&f.service, "account_removals"), 0);
    assert_eq!(count(&f.service, "secret_gc"), 0);
}
