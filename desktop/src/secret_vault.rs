//! One keychain item for every credential (macOS).
//!
//! The system keychain asks for permission per item, so one item per password
//! means one prompt per account. This store keeps a single item, `vault`, that
//! holds all secrets as JSON, reads it once per process and writes it back on
//! every change. Credentials written by earlier builds (one item per secret,
//! named by the secret's key) are moved in lazily the first time they are read.
//!
//! Design and trade-offs: `docs/architecture/secret-vault.md`.
// Only macOS builds use it; the logic is platform independent and its tests run
// everywhere against an in-memory backend.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use mailx_core::secrets::{SecretStore, SecretStoreError, SecretValue};
use serde::{Deserialize, Serialize};

/// Key of the vault inside the backing store. Real secret keys start with
/// `mailx/`, so the two can never collide.
const VAULT_KEY: &str = "vault";
const VERSION: u32 = 1;

#[derive(Clone, Default)]
struct Vault {
    secrets: HashMap<String, SecretValue>,
    /// Keys copied in from the per-item store whose old item still exists. They
    /// are removed the next time the vault is loaded, never in the same run.
    migrated: Vec<String>,
}

#[derive(Deserialize)]
struct VaultIn {
    version: u32,
    #[serde(default)]
    secrets: HashMap<String, SecretValue>,
    #[serde(default)]
    migrated: Vec<String>,
}

#[derive(Serialize)]
struct VaultOut<'a> {
    version: u32,
    secrets: BTreeMap<&'a str, &'a str>,
    migrated: &'a [String],
}

fn decode(raw: &SecretValue) -> Result<Vault, SecretStoreError> {
    // An unreadable or newer vault is an error, never an empty one: treating it
    // as empty would let the next write destroy every stored password.
    let parsed: VaultIn =
        serde_json::from_str(raw.expose_secret()).map_err(|_| SecretStoreError::Failed)?;
    if parsed.version != VERSION {
        return Err(SecretStoreError::Failed);
    }
    Ok(Vault {
        secrets: parsed.secrets,
        migrated: parsed.migrated,
    })
}

fn encode(vault: &Vault) -> Result<SecretValue, SecretStoreError> {
    let out = VaultOut {
        version: VERSION,
        secrets: vault
            .secrets
            .iter()
            .map(|(key, value)| (key.as_str(), value.expose_secret()))
            .collect(),
        migrated: &vault.migrated,
    };
    serde_json::to_string(&out)
        .map(SecretValue::new)
        .map_err(|_| SecretStoreError::Failed)
}

pub(crate) struct VaultSecretStore {
    /// The per-item store: holds the vault under [`VAULT_KEY`] and, from earlier
    /// builds, one item per secret.
    backend: Arc<dyn SecretStore>,
    /// `None` until the first successful read. A failed read is not cached, so a
    /// locked or denied keychain can be retried.
    state: Mutex<Option<Vault>>,
}

impl VaultSecretStore {
    pub(crate) fn new(backend: Arc<dyn SecretStore>) -> Self {
        Self {
            backend,
            state: Mutex::new(None),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Option<Vault>>, SecretStoreError> {
        self.state.lock().map_err(|_| SecretStoreError::Failed)
    }

    /// The vault, read from the backing store on first use.
    fn loaded<'a>(&self, slot: &'a mut Option<Vault>) -> Result<&'a mut Vault, SecretStoreError> {
        if slot.is_none() {
            *slot = Some(self.load()?);
        }
        slot.as_mut().ok_or(SecretStoreError::Failed)
    }

    fn load(&self) -> Result<Vault, SecretStoreError> {
        let Some(raw) = self.backend.get(VAULT_KEY)? else {
            return Ok(Vault::default());
        };
        let mut vault = decode(&raw)?;
        self.finish_migration(&mut vault);
        Ok(vault)
    }

    /// Removes the per-item copies of secrets a previous run moved into the
    /// vault. Best effort: whatever cannot be removed stays listed for the next
    /// start, and a failure never blocks reading credentials.
    fn finish_migration(&self, vault: &mut Vault) {
        if vault.migrated.is_empty() {
            return;
        }
        let listed = std::mem::take(&mut vault.migrated);
        let before = listed.len();
        let mut remaining = Vec::new();
        for key in listed {
            // Never delete the only copy: the vault must hold the value.
            if !vault.secrets.contains_key(&key) {
                continue;
            }
            if self.backend.delete(&key).is_err() {
                remaining.push(key);
            }
        }
        let changed = remaining.len() != before;
        vault.migrated = remaining;
        if changed {
            // If this write fails the old list is still on disk and the next
            // start repeats the (idempotent) deletes.
            let _ = self.persist(vault);
        }
    }

    fn persist(&self, vault: &Vault) -> Result<(), SecretStoreError> {
        self.backend.set(VAULT_KEY, &encode(vault)?)
    }
}

impl SecretStore for VaultSecretStore {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        let mut guard = self.lock()?;
        let vault = self.loaded(&mut guard)?;
        if let Some(value) = vault.secrets.get(key) {
            return Ok(Some(value.clone()));
        }
        // Not in the vault: it may still be a per-item credential from an
        // earlier build. A missing item answers without any prompt.
        let Some(value) = self.backend.get(key)? else {
            return Ok(None);
        };
        let mut next = vault.clone();
        next.secrets.insert(key.to_owned(), value.clone());
        if !next.migrated.iter().any(|listed| listed == key) {
            next.migrated.push(key.to_owned());
        }
        // The old item is still there, so a failed write loses nothing: answer
        // with the value and try again on the next read.
        if self.persist(&next).is_ok() {
            *vault = next;
        }
        Ok(Some(value))
    }

    fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        let mut guard = self.lock()?;
        let vault = self.loaded(&mut guard)?;
        let mut next = vault.clone();
        next.secrets.insert(key.to_owned(), value.clone());
        // Durable first; the cache only changes once the keychain has the data.
        self.persist(&next)?;
        *vault = next;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        let mut guard = self.lock()?;
        let vault = self.loaded(&mut guard)?;
        let in_vault = vault.secrets.contains_key(key);
        let listed = vault.migrated.iter().any(|item| item == key);
        // A per-item copy may exist unless the vault created this entry itself.
        // Removed first: if it fails, nothing has changed and a retry is safe.
        if listed || !in_vault {
            self.backend.delete(key)?;
        }
        if in_vault || listed {
            let mut next = vault.clone();
            next.secrets.remove(key);
            next.migrated.retain(|item| item != key);
            self.persist(&next)?;
            *vault = next;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const OLD_A: &str = "mailx/a/1";
    const OLD_B: &str = "mailx/b/2";

    #[derive(Default)]
    struct Backend {
        items: Mutex<HashMap<String, String>>,
        vault_reads: AtomicUsize,
        vault_writes: AtomicUsize,
        fail_writes: AtomicBool,
        fail_deletes: AtomicBool,
        read_error: Mutex<Option<SecretStoreError>>,
    }

    impl Backend {
        fn with_items(items: &[(&str, &str)]) -> Arc<Self> {
            let backend = Arc::new(Self::default());
            for (key, value) in items {
                backend
                    .items
                    .lock()
                    .unwrap()
                    .insert((*key).to_owned(), (*value).to_owned());
            }
            backend
        }

        fn has(&self, key: &str) -> bool {
            self.items.lock().unwrap().contains_key(key)
        }

        fn raw_vault(&self) -> Option<String> {
            self.items.lock().unwrap().get(VAULT_KEY).cloned()
        }
    }

    impl SecretStore for Backend {
        fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
            if key == VAULT_KEY {
                self.vault_reads.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(error) = *self.read_error.lock().unwrap() {
                return Err(error);
            }
            Ok(self
                .items
                .lock()
                .unwrap()
                .get(key)
                .map(|value| SecretValue::new(value.clone())))
        }

        fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
            if self.fail_writes.load(Ordering::SeqCst) {
                return Err(SecretStoreError::Unavailable);
            }
            if key == VAULT_KEY {
                self.vault_writes.fetch_add(1, Ordering::SeqCst);
            }
            self.items
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.expose_secret().to_owned());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
            if self.fail_deletes.load(Ordering::SeqCst) {
                return Err(SecretStoreError::Unavailable);
            }
            self.items.lock().unwrap().remove(key);
            Ok(())
        }
    }

    fn store(backend: &Arc<Backend>) -> VaultSecretStore {
        VaultSecretStore::new(backend.clone())
    }

    fn value(text: &str) -> SecretValue {
        SecretValue::new(text.to_owned())
    }

    /// The error of a `get`, which cannot be compared directly because a secret
    /// value deliberately has no equality.
    fn get_error(store: &VaultSecretStore, key: &str) -> SecretStoreError {
        store.get(key).expect_err("get should have failed")
    }

    fn read(store: &VaultSecretStore, key: &str) -> Option<String> {
        store
            .get(key)
            .unwrap()
            .map(|secret| secret.expose_secret().to_owned())
    }

    #[test]
    fn secrets_round_trip_through_a_single_backend_item() {
        let backend = Backend::with_items(&[]);
        let vault = store(&backend);
        vault.set("mailx/x/1", &value("one")).unwrap();
        vault.set("mailx/y/2", &value("two")).unwrap();
        assert_eq!(read(&vault, "mailx/x/1").as_deref(), Some("one"));
        assert_eq!(read(&vault, "mailx/y/2").as_deref(), Some("two"));
        assert_eq!(read(&vault, "mailx/none/3"), None);
        // Everything lives in one item, so a keychain only ever asks about one.
        let items = backend.items.lock().unwrap();
        assert_eq!(items.keys().collect::<Vec<_>>(), [VAULT_KEY]);
    }

    #[test]
    fn many_reads_touch_the_vault_item_once_per_process() {
        let backend = Backend::with_items(&[]);
        let writer = store(&backend);
        for index in 0..5 {
            writer
                .set(&format!("mailx/a/{index}"), &value("secret"))
                .unwrap();
        }
        let before = backend.vault_reads.load(Ordering::SeqCst);

        let fresh = store(&backend);
        for index in 0..5 {
            assert!(read(&fresh, &format!("mailx/a/{index}")).is_some());
        }
        assert_eq!(read(&fresh, "mailx/none/9"), None);
        assert_eq!(backend.vault_reads.load(Ordering::SeqCst) - before, 1);
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let backend = Backend::with_items(&[]);
        let vault = store(&backend);
        vault.set("mailx/x/1", &value("old")).unwrap();
        backend.fail_writes.store(true, Ordering::SeqCst);
        assert_eq!(
            vault.set("mailx/x/1", &value("new")),
            Err(SecretStoreError::Unavailable)
        );
        assert_eq!(
            vault.set("mailx/y/2", &value("other")),
            Err(SecretStoreError::Unavailable)
        );
        assert_eq!(read(&vault, "mailx/x/1").as_deref(), Some("old"));
        assert_eq!(read(&vault, "mailx/y/2"), None);
    }

    #[test]
    fn concurrent_writers_do_not_lose_updates() {
        let backend = Backend::with_items(&[]);
        let vault = Arc::new(store(&backend));
        let threads: Vec<_> = (0..16)
            .map(|index| {
                let vault = vault.clone();
                std::thread::spawn(move || {
                    vault
                        .set(&format!("mailx/t/{index}"), &value(&format!("v{index}")))
                        .unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let fresh = store(&backend);
        for index in 0..16 {
            assert_eq!(
                read(&fresh, &format!("mailx/t/{index}")),
                Some(format!("v{index}"))
            );
        }
    }

    #[test]
    fn older_per_item_credentials_move_in_when_first_read() {
        let backend = Backend::with_items(&[(OLD_A, "pw-a"), (OLD_B, "pw-b")]);
        let vault = store(&backend);
        assert_eq!(read(&vault, OLD_A).as_deref(), Some("pw-a"));
        assert_eq!(read(&vault, OLD_B).as_deref(), Some("pw-b"));
        // Copied, not moved: the old items stay for one more start.
        assert!(backend.has(OLD_A) && backend.has(OLD_B));
        let raw = backend.raw_vault().unwrap();
        assert!(raw.contains(OLD_A) && raw.contains(OLD_B));
        let reopened = store(&backend);
        assert_eq!(read(&reopened, OLD_A).as_deref(), Some("pw-a"));
    }

    #[test]
    fn the_old_items_go_on_the_next_start_and_values_survive() {
        let backend = Backend::with_items(&[(OLD_A, "pw-a"), (OLD_B, "pw-b")]);
        let first = store(&backend);
        read(&first, OLD_A);
        read(&first, OLD_B);
        assert!(backend.has(OLD_A), "no cleanup in the run that migrated");

        let second = store(&backend);
        assert_eq!(read(&second, OLD_A).as_deref(), Some("pw-a"));
        assert!(!backend.has(OLD_A) && !backend.has(OLD_B));
        assert!(!backend.raw_vault().unwrap().contains("\"migrated\":[\""));
        let third = store(&backend);
        assert_eq!(read(&third, OLD_B).as_deref(), Some("pw-b"));
    }

    #[test]
    fn a_failed_cleanup_is_retried_and_never_blocks_reading() {
        let backend = Backend::with_items(&[(OLD_A, "pw-a")]);
        read(&store(&backend), OLD_A);
        backend.fail_deletes.store(true, Ordering::SeqCst);
        assert_eq!(read(&store(&backend), OLD_A).as_deref(), Some("pw-a"));
        assert!(backend.has(OLD_A));
        backend.fail_deletes.store(false, Ordering::SeqCst);
        assert_eq!(read(&store(&backend), OLD_A).as_deref(), Some("pw-a"));
        assert!(!backend.has(OLD_A));
    }

    #[test]
    fn cleanup_never_deletes_the_only_copy() {
        let backend = Backend::with_items(&[(OLD_A, "pw-a")]);
        // A vault that lists a key it does not hold.
        backend.items.lock().unwrap().insert(
            VAULT_KEY.into(),
            format!(r#"{{"version":1,"secrets":{{}},"migrated":["{OLD_A}"]}}"#),
        );
        let vault = store(&backend);
        assert_eq!(read(&vault, OLD_A).as_deref(), Some("pw-a"));
        assert!(backend.has(OLD_A));
    }

    #[test]
    fn a_migration_that_cannot_be_saved_still_returns_the_password() {
        let backend = Backend::with_items(&[(OLD_A, "pw-a")]);
        backend.fail_writes.store(true, Ordering::SeqCst);
        let vault = store(&backend);
        assert_eq!(read(&vault, OLD_A).as_deref(), Some("pw-a"));
        assert!(backend.has(OLD_A));
        assert!(backend.raw_vault().is_none());
        backend.fail_writes.store(false, Ordering::SeqCst);
        assert_eq!(read(&vault, OLD_A).as_deref(), Some("pw-a"));
        assert!(backend.raw_vault().unwrap().contains(OLD_A));
    }

    #[test]
    fn delete_removes_every_copy_and_is_idempotent() {
        let backend = Backend::with_items(&[(OLD_A, "pw-a"), (OLD_B, "pw-b")]);
        let vault = store(&backend);
        read(&vault, OLD_A);
        vault.set("mailx/c/3", &value("pw-c")).unwrap();
        for key in [OLD_A, OLD_B, "mailx/c/3", "mailx/none/9"] {
            vault.delete(key).unwrap();
            vault.delete(key).unwrap();
            assert_eq!(read(&vault, key), None, "{key} came back");
        }
        assert!(!backend.has(OLD_A) && !backend.has(OLD_B));
        assert_eq!(read(&store(&backend), OLD_A), None);
    }

    #[test]
    fn a_failed_delete_leaves_the_secret_in_place() {
        let backend = Backend::with_items(&[(OLD_A, "pw-a")]);
        let vault = store(&backend);
        read(&vault, OLD_A);
        backend.fail_deletes.store(true, Ordering::SeqCst);
        assert!(vault.delete(OLD_A).is_err());
        assert_eq!(read(&vault, OLD_A).as_deref(), Some("pw-a"));
    }

    #[test]
    fn an_unreadable_vault_is_an_error_and_is_never_overwritten() {
        for bad in [
            "not json",
            r#"{"version":2,"secrets":{}}"#,
            r#"{"secrets":{}}"#,
        ] {
            let backend = Backend::with_items(&[(VAULT_KEY, bad)]);
            let vault = store(&backend);
            assert_eq!(
                get_error(&vault, "mailx/x/1"),
                SecretStoreError::Failed,
                "{bad}"
            );
            assert_eq!(
                vault.set("mailx/x/1", &value("new")),
                Err(SecretStoreError::Failed)
            );
            assert_eq!(backend.raw_vault().as_deref(), Some(bad));
        }
    }

    #[test]
    fn a_locked_keychain_is_reported_and_retried() {
        let backend = Backend::with_items(&[]);
        store(&backend).set("mailx/x/1", &value("pw")).unwrap();
        let vault = store(&backend);
        *backend.read_error.lock().unwrap() = Some(SecretStoreError::Locked);
        assert_eq!(get_error(&vault, "mailx/x/1"), SecretStoreError::Locked);
        assert_eq!(get_error(&vault, "mailx/x/1"), SecretStoreError::Locked);
        *backend.read_error.lock().unwrap() = None;
        assert_eq!(read(&vault, "mailx/x/1").as_deref(), Some("pw"));
    }

    /// A pre-vault install: three accounts, each with a password stored as its
    /// own backend item. Starting the core lists every account, and listing reads
    /// every credential, so the whole migration happens in that one pass.
    fn legacy_install(accounts: usize) -> (tempfile::TempDir, Arc<Backend>) {
        let directory = tempfile::TempDir::new().unwrap();
        let backend = Backend::with_items(&[]);
        // Created through the plain per-item store, as earlier builds did.
        let mut service =
            mailx_core::AccountService::open(directory.path(), backend.clone()).unwrap();
        for index in 0..accounts {
            service
                .create(
                    serde_json::from_value(serde_json::json!({
                        "requestId": uuid::Uuid::new_v4().to_string(),
                        "config": {
                            "displayName": format!("Account {index}"),
                            "emailAddress": format!("user{index}@example.com"),
                            "providerId": null,
                            "imap": {"host": "imap.example.com", "port": 993, "tls": "implicit", "username": format!("u{index}")},
                            "smtp": {"host": "smtp.example.com", "port": 587, "tls": "startTls", "username": format!("u{index}")},
                            "authentication": {"kind": "password", "mode": "shared",
                                "password": {"action": "replace", "value": format!("synthetic-secret-{index}")}},
                            "route": {"kind": "direct"}
                        }
                    }))
                    .unwrap(),
                )
                .unwrap();
        }
        drop(service);
        (directory, backend)
    }

    fn per_item_keys(backend: &Backend) -> Vec<String> {
        let mut keys: Vec<String> = backend
            .items
            .lock()
            .unwrap()
            .keys()
            .filter(|key| key.starts_with("mailx/"))
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn starting_with_an_older_install_moves_every_credential_in_one_pass() {
        let (directory, backend) = legacy_install(3);
        let legacy = per_item_keys(&backend);
        assert_eq!(legacy.len(), 3);
        assert!(backend.raw_vault().is_none());

        // First start with the vault: listing the accounts reads every password.
        let first = Arc::new(store(&backend));
        let service = mailx_core::AccountService::open(directory.path(), first.clone()).unwrap();
        let accounts = service.list().unwrap();
        assert_eq!(accounts.len(), 3);
        assert!(accounts.iter().all(|account| {
            account.authentication.imap.password_configured
                && account.authentication.smtp.password_configured
        }));
        drop(service);
        let raw = backend.raw_vault().expect("the vault was written");
        for key in &legacy {
            assert!(raw.contains(key.as_str()), "{key} missing from the vault");
        }
        assert_eq!(
            per_item_keys(&backend),
            legacy,
            "old items stay for one more start"
        );

        // Second start: the vault is read once, the old items are cleaned up.
        let before = backend.vault_reads.load(Ordering::SeqCst);
        let second = Arc::new(store(&backend));
        let service = mailx_core::AccountService::open(directory.path(), second).unwrap();
        assert!(
            service
                .list()
                .unwrap()
                .iter()
                .all(|account| { account.authentication.imap.password_configured })
        );
        assert_eq!(backend.vault_reads.load(Ordering::SeqCst) - before, 1);
        assert!(
            per_item_keys(&backend).is_empty(),
            "old items were not removed"
        );
    }

    #[test]
    fn a_fresh_install_never_creates_per_item_credentials() {
        let directory = tempfile::TempDir::new().unwrap();
        let backend = Backend::with_items(&[]);
        let mut service =
            mailx_core::AccountService::open(directory.path(), Arc::new(store(&backend))).unwrap();
        service
            .create(
                serde_json::from_value(serde_json::json!({
                    "requestId": uuid::Uuid::new_v4().to_string(),
                    "config": {
                        "displayName": "A", "emailAddress": "a@example.com", "providerId": null,
                        "imap": {"host": "imap.example.com", "port": 993, "tls": "implicit", "username": "a"},
                        "smtp": {"host": "smtp.example.com", "port": 587, "tls": "startTls", "username": "a"},
                        "authentication": {"kind": "password", "mode": "shared",
                            "password": {"action": "replace", "value": "synthetic-secret"}},
                        "route": {"kind": "direct"}
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        assert!(per_item_keys(&backend).is_empty());
        assert!(backend.raw_vault().unwrap().contains("synthetic-secret"));
    }

    #[test]
    fn debug_output_never_shows_a_password() {
        let backend = Backend::with_items(&[]);
        let vault = store(&backend);
        vault
            .set("mailx/x/1", &value("synthetic-password"))
            .unwrap();
        let shown = format!("{:?}", vault.get("mailx/x/1").unwrap());
        assert!(!shown.contains("synthetic-password"), "{shown}");
    }
}
