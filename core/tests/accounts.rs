use mailx_core::{
    secrets::{SecretStore, SecretStoreError, SecretValue},
    *,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Default)]
struct MemorySecrets {
    state: Mutex<SecretState>,
}
#[derive(Default)]
struct SecretState {
    entries: HashMap<String, SecretValue>,
    set_count: usize,
    fail_set: Option<(usize, bool)>,
    get_error: Option<SecretStoreError>,
    get_count: usize,
    fail_get: Option<usize>,
    fail_delete: bool,
}
impl MemorySecrets {
    fn len(&self) -> usize {
        self.state.lock().unwrap().entries.len()
    }
    fn fail_next_set(&self, offset: usize, after_persist: bool) {
        let mut state = self.state.lock().unwrap();
        state.fail_set = Some((state.set_count + offset, after_persist));
    }
    fn fail_next_get(&self, offset: usize) {
        let mut state = self.state.lock().unwrap();
        state.fail_get = Some(state.get_count + offset);
    }
}
impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        let mut state = self.state.lock().unwrap();
        state.get_count += 1;
        if let Some(error) = state.get_error {
            return Err(error);
        }
        if state.fail_get == Some(state.get_count) {
            return Err(SecretStoreError::Unavailable);
        }
        Ok(state.entries.get(key).cloned())
    }
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        let mut state = self.state.lock().unwrap();
        state.set_count += 1;
        if state.fail_set == Some((state.set_count, false)) {
            return Err(SecretStoreError::Unavailable);
        }
        state.entries.insert(key.into(), value.clone());
        if state.fail_set == Some((state.set_count, true)) {
            return Err(SecretStoreError::Unavailable);
        }
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        let mut state = self.state.lock().unwrap();
        if state.fail_delete {
            return Err(SecretStoreError::Unavailable);
        }
        state.entries.remove(key);
        Ok(())
    }
}

fn config() -> Value {
    json!({"displayName":"Personal","emailAddress":"alice@example.com","providerId":null,
        "imap":{"host":"imap.example.com","port":993,"tls":"implicit","username":"alice-imap"},
        "smtp":{"host":"smtp.example.com","port":587,"tls":"startTls","username":"alice-smtp"},
        "authentication":{"kind":"password","mode":"shared","password":{"action":"replace","value":"test-mail-secret"}},
        "route":{"kind":"direct"}})
}
fn create_request(id: &str, value: Value) -> CreateAccountRequest {
    serde_json::from_value(json!({"requestId":id,"config":value})).unwrap()
}
fn create(service: &mut AccountService) -> Account {
    service
        .create(create_request(&Uuid::new_v4().to_string(), config()))
        .unwrap()
}
fn keep_config() -> Value {
    let mut value = config();
    value["authentication"]["password"] = json!({"action":"keep"});
    value
}
/// Builds a connection update from a full account config; the profile fields
/// belong to a separate request.
fn update_request(account: &Account, mut value: Value) -> UpdateConnectionRequest {
    let object = value.as_object_mut().unwrap();
    object.remove("displayName");
    object.remove("color");
    object.remove("senderName");
    object.remove("useProviderIcon");
    serde_json::from_value(
        json!({"accountId":account.id,"expectedRevision":account.revision,"connection":value}),
    )
    .unwrap()
}
fn setup() -> (TempDir, Arc<MemorySecrets>, AccountService) {
    let directory = TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let service = AccountService::open(directory.path(), secrets.clone()).unwrap();
    (directory, secrets, service)
}
fn db(path: &Path) -> Connection {
    let db = Connection::open(path.join("mailx.sqlite3")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db
}
fn count(database: &Connection, table: &str) -> u32 {
    database
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn persists_config_across_restart_without_disclosing_secrets() {
    let (directory, secrets, mut service) = setup();
    let account = create(&mut service);
    assert_eq!(account.connection_status, ConnectionStatus::Unverified);
    assert_eq!(account.authentication.mode, AuthenticationMode::Shared);
    assert_eq!(
        account.authentication.imap.id,
        account.authentication.smtp.id
    );
    assert_eq!(account.imap.username, "alice-imap");
    assert_eq!(account.smtp.username, "alice-smtp");
    assert_eq!(secrets.len(), 1);
    let output = serde_json::to_string(&account).unwrap();
    assert!(!output.contains("test-mail-secret"));
    assert!(!output.contains("secretRef"));
    let debug = format!(
        "{:?}",
        create_request(&Uuid::new_v4().to_string(), config())
    );
    assert!(debug.contains("REDACTED"));
    assert!(!debug.contains("test-mail-secret"));
    let mut paths = vec![directory.path().to_path_buf()];
    while let Some(path) = paths.pop() {
        if path.is_dir() {
            paths.extend(
                std::fs::read_dir(path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
            continue;
        }
        // The empty instance lock is held open, and Windows locks are mandatory, so it cannot be read.
        if path.file_name().is_some_and(|name| name == "mailx.lock") {
            continue;
        }
        let bytes = std::fs::read(path).unwrap();
        assert!(
            !bytes
                .windows(b"test-mail-secret".len())
                .any(|part| part == b"test-mail-secret")
        );
    }
    drop(service);
    let service = AccountService::open(directory.path(), secrets).unwrap();
    assert_eq!(service.list().unwrap(), vec![account]);
}

#[test]
fn mail_usernames_default_independently_and_preserve_explicit_case() {
    let (_directory, _secrets, mut service) = setup();
    for (imap, smtp, expected_imap, expected_smtp) in [
        ("", "", "Alice@example.com", "Alice@example.com"),
        ("", "smtp-user", "Alice@example.com", "smtp-user"),
        ("imap-user", "", "imap-user", "Alice@example.com"),
        (" \t ", "\u{3000}", "Alice@example.com", "Alice@example.com"),
        (
            "\u{feff}",
            "\u{85}",
            "Alice@example.com",
            "Alice@example.com",
        ),
        (
            "\u{feff} \u{85}\u{3000}",
            "\u{85}\t\u{feff}",
            "Alice@example.com",
            "Alice@example.com",
        ),
        ("  IMAP-Alice  ", " SMTP-Alice ", "IMAP-Alice", "SMTP-Alice"),
        (
            "\u{feff}\u{85} IMAP-Alice \u{85}\u{feff}",
            "\u{85}\u{feff} SMTP-Alice \u{feff}\u{85}",
            "IMAP-Alice",
            "SMTP-Alice",
        ),
    ] {
        let mut input = config();
        input["emailAddress"] = json!("  Alice@example.com ");
        input["imap"]["username"] = json!(imap);
        input["smtp"]["username"] = json!(smtp);
        let account = service
            .create(create_request(&Uuid::new_v4().to_string(), input))
            .unwrap();
        assert_eq!(account.email_address, "Alice@example.com");
        assert_eq!(account.imap.username, expected_imap);
        assert_eq!(account.smtp.username, expected_smtp);
        assert_eq!(account.authentication.imap.trust, CredentialTrust::Trusted);
        assert_eq!(account.authentication.smtp.trust, CredentialTrust::Trusted);
        assert_eq!(service.get(&account.id).unwrap(), account);
    }
}

#[test]
fn default_and_explicit_mail_usernames_have_the_same_create_identity() {
    let (_directory, secrets, mut service) = setup();
    let request_id = Uuid::new_v4().to_string();
    let mut input = config();
    input["imap"]["username"] = json!("\u{feff}\u{85}");
    input["smtp"]["username"] = json!(" \u{85}\u{feff} ");
    let account = service
        .create(create_request(&request_id, input.clone()))
        .unwrap();
    input["imap"]["username"] = input["emailAddress"].clone();
    input["smtp"]["username"] = json!(" alice@example.com ");
    assert_eq!(
        service.create(create_request(&request_id, input)).unwrap(),
        account
    );
    assert_eq!(service.list().unwrap().len(), 1);
    assert_eq!(secrets.len(), 1);
}

#[test]
fn default_mail_usernames_keep_trust_until_the_effective_identity_changes() {
    let (_directory, secrets, mut service) = setup();
    let mut input = config();
    input["imap"]["username"] = json!("");
    input["smtp"]["username"] = json!("");
    let original = service
        .create(create_request(&Uuid::new_v4().to_string(), input.clone()))
        .unwrap();
    input["authentication"]["password"] = json!({"action":"keep"});
    input["imap"]["username"] = json!(" ");
    let same = service
        .update_connection(update_request(&original, input.clone()))
        .unwrap();
    assert_eq!(same.mailbox_source_id, original.mailbox_source_id);
    assert_eq!(same.authentication, original.authentication);

    // Saved usernames are concrete values, so changing only the display email
    // leaves both logins and their trust untouched.
    input["emailAddress"] = json!("other@example.com");
    input["imap"]["username"] = json!(same.imap.username);
    input["smtp"]["username"] = json!(same.smtp.username);
    let explicit = service
        .update_connection(update_request(&same, input.clone()))
        .unwrap();
    assert_eq!(explicit.mailbox_source_id, original.mailbox_source_id);
    assert_eq!(explicit.authentication, original.authentication);

    input["imap"]["username"] = json!("");
    input["smtp"]["username"] = json!(" ");
    let changed = service
        .update_connection(update_request(&explicit, input.clone()))
        .unwrap();
    assert_eq!(changed.imap.username, "other@example.com");
    assert_eq!(changed.smtp.username, "other@example.com");
    assert_ne!(changed.mailbox_source_id, original.mailbox_source_id);
    assert_eq!(
        changed.authentication.imap.trust,
        CredentialTrust::NeedsConfirmation
    );
    assert_eq!(
        changed.authentication.smtp.trust,
        CredentialTrust::NeedsConfirmation
    );
    assert_eq!(secrets.len(), 1);

    input["authentication"]["password"] = json!({"action":"replace","value":"confirmed-password"});
    let confirmed = service
        .update_connection(update_request(&changed, input))
        .unwrap();
    assert_eq!(confirmed.mailbox_source_id, changed.mailbox_source_id);
    assert_eq!(
        confirmed.authentication.imap.trust,
        CredentialTrust::Trusted
    );
    assert_eq!(
        confirmed.authentication.smtp.trust,
        CredentialTrust::Trusted
    );
}

#[test]
fn mail_username_normalization_still_rejects_invalid_nonempty_values() {
    let (directory, secrets, mut service) = setup();
    for protocol in ["imap", "smtp"] {
        for username in ["alice\nother".to_owned(), "a".repeat(321)] {
            let mut input = config();
            input[protocol]["username"] = json!(username);
            assert_eq!(
                service
                    .create(create_request(&Uuid::new_v4().to_string(), input))
                    .unwrap_err(),
                AppError::validation(&format!("{protocol}.username"))
            );
        }
    }
    assert_eq!(count(&db(directory.path()), "accounts"), 0);
    assert_eq!(secrets.len(), 0);
}

#[test]
fn identical_creates_are_idempotent_but_changes_and_secret_changes_conflict() {
    let (_directory, _secrets, mut service) = setup();
    let request = Uuid::new_v4().to_string();
    let first = service.create(create_request(&request, config())).unwrap();
    assert_eq!(
        first,
        service.create(create_request(&request, config())).unwrap()
    );
    assert_eq!(service.list().unwrap().len(), 1);
    let mut changed = config();
    changed["authentication"]["password"]["value"] = json!("different-password");
    assert_eq!(
        service
            .create(create_request(&request, changed))
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    );
    let mut changed = config();
    changed["displayName"] = json!("Other");
    assert_eq!(
        service
            .create(create_request(&request, changed))
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    );
    let updated = service
        .update_connection(update_request(&first, keep_config()))
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(
        service
            .create(create_request(&request, config()))
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    );
}

#[test]
fn shared_and_separate_authentication_preserve_ownership_and_versions() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    let mut separate = keep_config();
    separate["authentication"] = json!({"kind":"password","mode":"separate","imapPassword":{"action":"keep"},"smtpPassword":{"action":"keep"}});
    let split = service
        .update_connection(update_request(&original, separate.clone()))
        .unwrap();
    assert_ne!(split.authentication.imap.id, split.authentication.smtp.id);
    assert_eq!(split.authentication.imap.generation, 1);
    assert_eq!(split.authentication.smtp.trust, CredentialTrust::Trusted);
    assert_eq!(secrets.len(), 2);
    separate["authentication"]["smtpPassword"] =
        json!({"action":"replace","value":"new-smtp-password"});
    let rotated = service
        .update_connection(update_request(&split, separate))
        .unwrap();
    assert_eq!(rotated.authentication.smtp.generation, 2);
    assert_eq!(rotated.authentication.imap.generation, 1);
    assert_eq!(rotated.mailbox_source_id, original.mailbox_source_id);
    assert_eq!(secrets.len(), 2);
    let merged = service
        .update_connection(update_request(&rotated, keep_config()))
        .unwrap();
    assert_eq!(merged.authentication.imap.id, merged.authentication.smtp.id);
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "auth_profiles"), 1);
}

#[test]
fn new_target_requires_credential_confirmation_and_new_mailbox_source() {
    let (_directory, _secrets, mut service) = setup();
    let original = create(&mut service);
    let mut moved = keep_config();
    moved["imap"]["host"] = json!("new.example.com");
    let pending = service
        .update_connection(update_request(&original, moved.clone()))
        .unwrap();
    assert_eq!(pending.id, original.id);
    assert_ne!(pending.mailbox_source_id, original.mailbox_source_id);
    assert_eq!(
        pending.authentication.imap.trust,
        CredentialTrust::NeedsConfirmation
    );
    assert_eq!(pending.authentication.smtp.trust, CredentialTrust::Trusted);
    moved["authentication"]["password"] =
        json!({"action":"replace","value":"replacement-password"});
    let confirmed = service
        .update_connection(update_request(&pending, moved))
        .unwrap();
    assert_eq!(
        confirmed.authentication.imap.trust,
        CredentialTrust::Trusted
    );
    assert_eq!(confirmed.mailbox_source_id, pending.mailbox_source_id);
    assert_eq!(
        service
            .update_connection(update_request(&original, keep_config()))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[test]
fn tls_change_does_not_expand_trust_or_change_mailbox_identity() {
    let (_directory, _secrets, mut service) = setup();
    let account = create(&mut service);
    let mut config = keep_config();
    config["imap"]["tls"] = json!("startTls");
    let updated = service
        .update_connection(update_request(&account, config))
        .unwrap();
    assert_eq!(updated.mailbox_source_id, account.mailbox_source_id);
    assert_eq!(
        updated.authentication.imap.trust,
        CredentialTrust::NeedsConfirmation
    );
}

#[test]
fn rejects_cross_account_links_and_unknown_wire_fields() {
    let (directory, _secrets, mut service) = setup();
    let first = create(&mut service);
    let second = create(&mut service);
    let database = db(directory.path());
    assert!(
        database
            .execute(
                "UPDATE mail_connections SET auth_profile_id=?1 WHERE account_id=?2",
                params![second.authentication.imap.id, first.id]
            )
            .is_err()
    );
    assert_eq!(service.get(&first.id).unwrap(), first);
    let mut malicious = config();
    malicious["authentication"]["secretRef"] = json!("arbitrary-private-reference");
    assert!(serde_json::from_value::<AccountConfigInput>(malicious).is_err());
    let mut malicious = config();
    malicious["imap"]["authProfileId"] = json!(second.authentication.imap.id);
    assert!(serde_json::from_value::<AccountConfigInput>(malicious).is_err());
    let mut malicious = json!({"requestId":Uuid::new_v4().to_string(),"config":config()});
    malicious["principal"] = json!("admin");
    assert!(serde_json::from_value::<CreateAccountRequest>(malicious).is_err());
}

#[test]
fn oauth_and_invalid_parameters_fail_before_persistent_side_effects() {
    let (directory, secrets, mut service) = setup();
    let mut oauth = config();
    oauth["authentication"] = json!({"kind":"oauth2","configuration":{"providerId":"google","issuer":"https://accounts.google.com","clientRegistrationId":"desktop","authorizationSubject":null,"targetMailboxResource":null}});
    assert_eq!(
        service
            .create(create_request(&Uuid::new_v4().to_string(), oauth))
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    for value in ["", "********", "••••••••"] {
        let mut invalid = config();
        invalid["authentication"]["password"]["value"] = json!(value);
        assert_eq!(
            service
                .create(create_request(&Uuid::new_v4().to_string(), invalid))
                .unwrap_err()
                .code,
            ErrorCode::Validation
        );
    }
    let mut missing = config();
    missing["authentication"]["password"] = json!({"action":"keep"});
    assert_eq!(
        service
            .create(create_request(&Uuid::new_v4().to_string(), missing))
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
    assert_eq!(secrets.len(), 0);
    assert_eq!(count(&db(directory.path()), "accounts"), 0);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

#[test]
fn route_variants_persist_and_proxy_credentials_obey_trust() {
    let (directory, secrets, mut service) = setup();
    let initial = create(&mut service);
    let mut input = keep_config();
    input["route"] = json!({"kind":"inherit"});
    let inherited = service
        .update_connection(update_request(&initial, input.clone()))
        .unwrap();
    assert_eq!(inherited.default_route_revision, Some(1));
    assert_eq!(inherited.route_revision, 2);
    input["route"] = json!({"kind":"custom","proxyKind":"socks5","host":"proxy.example.com","port":1080,"dns":"proxy","authentication":{"username":"proxy-user","password":{"action":"replace","value":"proxy-password"}}});
    let proxy = service
        .update_connection(update_request(&inherited, input.clone()))
        .unwrap();
    assert_eq!(proxy.route_revision, 3);
    assert_eq!(secrets.len(), 2);
    input["route"]["proxyKind"] = json!("httpConnect");
    input["route"]["authentication"]["password"] = json!({"action":"keep"});
    let moved = service
        .update_connection(update_request(&proxy, input.clone()))
        .unwrap();
    assert!(matches!(
        &moved.route,
        RouteConfig::Custom {
            authentication: Some(ProxyAuthenticationSummary {
                trust: CredentialTrust::NeedsConfirmation,
                ..
            }),
            ..
        }
    ));
    assert_eq!(moved.route_revision, 4);
    drop(service);
    let mut service = AccountService::open(directory.path(), secrets.clone()).unwrap();
    assert_eq!(service.get(&moved.id).unwrap(), moved);
    for host in [
        "http://proxy.example.com",
        "user:password@proxy.example.com",
        "proxy.example.com/path",
        "proxy.example.com?password=x",
    ] {
        input["route"]["host"] = json!(host);
        assert_eq!(
            service
                .update_connection(update_request(&moved, input.clone()))
                .unwrap_err()
                .code,
            ErrorCode::Validation
        );
    }
    let direct = service
        .update_connection(update_request(&moved, keep_config()))
        .unwrap();
    assert_eq!(direct.route, RouteConfig::Direct);
    assert_eq!(secrets.len(), 1);
}

#[test]
fn same_directory_second_writer_is_rejected_but_lock_releases_on_drop() {
    let (directory, secrets, service) = setup();
    assert!(matches!(
        AccountService::open(directory.path(), secrets.clone()),
        Err(AppError {
            code: ErrorCode::AlreadyOpen,
            ..
        })
    ));
    drop(service);
    assert!(AccountService::open(directory.path(), secrets).is_ok());
}

#[test]
fn future_schema_and_checksum_mismatch_preserve_data() {
    let (directory, secrets, mut service) = setup();
    let account = create(&mut service);
    drop(service);
    let database = db(directory.path());
    let current: i64 = database
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    // A version this build does not know, as written by a newer MailX.
    database
        .pragma_update(None, "user_version", current + 1)
        .unwrap();
    assert!(matches!(
        AccountService::open(directory.path(), secrets.clone()),
        Err(AppError {
            code: ErrorCode::StorageIncompatible,
            ..
        })
    ));
    assert_eq!(count(&database, "accounts"), 1);
    database
        .pragma_update(None, "user_version", current)
        .unwrap();
    database
        .execute("UPDATE schema_migrations SET checksum='modified'", [])
        .unwrap();
    assert!(matches!(
        AccountService::open(directory.path(), secrets),
        Err(AppError {
            code: ErrorCode::StorageIncompatible,
            ..
        })
    ));
    assert_eq!(
        database
            .query_row("SELECT id FROM accounts", [], |r| r.get::<_, String>(0))
            .unwrap(),
        account.id
    );
}

#[test]
fn unversioned_existing_database_is_not_rebuilt() {
    let directory = TempDir::new().unwrap();
    let database = db(directory.path());
    database
        .execute("CREATE TABLE unrelated(value TEXT)", [])
        .unwrap();
    database
        .execute("INSERT INTO unrelated VALUES ('keep-me')", [])
        .unwrap();
    assert!(matches!(
        AccountService::open(directory.path(), Arc::new(MemorySecrets::default())),
        Err(AppError {
            code: ErrorCode::StorageIncompatible,
            ..
        })
    ));
    assert_eq!(
        database
            .query_row("SELECT value FROM unrelated", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "keep-me"
    );
}

#[test]
fn recovery_commits_fully_persisted_secret_after_lost_write_response() {
    let (directory, secrets, mut service) = setup();
    let request = Uuid::new_v4().to_string();
    secrets.fail_next_set(1, true);
    let error = service
        .create(create_request(&request, config()))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SecretStoreUnavailable);
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "accounts"), 0);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 1);
    drop(service);
    let mut reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    let account = reopened.create(create_request(&request, config())).unwrap();
    assert_eq!(reopened.list().unwrap(), vec![account]);
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

#[test]
fn partial_credential_update_rolls_back_new_entries_and_keeps_old_config() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    let mut input = config();
    input["authentication"] = json!({"kind":"password","mode":"separate","imapPassword":{"action":"replace","value":"new-imap"},"smtpPassword":{"action":"replace","value":"new-smtp"}});
    secrets.fail_next_set(2, false);
    assert_eq!(
        service
            .update_connection(update_request(&original, input))
            .unwrap_err()
            .code,
        ErrorCode::SecretStoreUnavailable
    );
    assert_eq!(secrets.len(), 2);
    drop(service);
    let reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    assert_eq!(reopened.get(&original.id).unwrap(), original);
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

#[test]
fn unavailable_secret_store_does_not_discard_pending_or_existing_credentials() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    secrets.fail_next_set(1, true);
    assert!(
        service
            .update_connection(update_request(&original, config()))
            .is_err()
    );
    assert_eq!(secrets.len(), 2);
    drop(service);
    secrets.state.lock().unwrap().get_error = Some(SecretStoreError::Locked);
    assert!(matches!(
        AccountService::open(directory.path(), secrets.clone()),
        Err(AppError {
            code: ErrorCode::SecretStoreLocked,
            ..
        })
    ));
    assert_eq!(secrets.len(), 2);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 1);
    secrets.state.lock().unwrap().get_error = None;
    let reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    assert_eq!(reopened.get(&original.id).unwrap().revision, 2);
    assert_eq!(secrets.len(), 1);
}

#[test]
fn cleanup_failure_is_retried_without_deleting_new_valid_secret() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    secrets.state.lock().unwrap().fail_delete = true;
    let changed = service
        .update_connection(update_request(&original, config()))
        .unwrap();
    assert_eq!(changed.revision, 2);
    assert_eq!(secrets.len(), 2);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 1);
    drop(service);
    let reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    assert_eq!(reopened.get(&changed.id).unwrap(), changed);
    assert_eq!(secrets.len(), 2);
    drop(reopened);
    secrets.state.lock().unwrap().fail_delete = false;
    let reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    assert_eq!(reopened.get(&changed.id).unwrap(), changed);
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

#[test]
fn failed_database_commit_preserves_intent_and_old_config_then_recovers() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    let database = db(directory.path());
    database.execute_batch("CREATE TRIGGER fail_account_update BEFORE UPDATE ON accounts BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert_eq!(
        service
            .update_connection(update_request(&original, config()))
            .unwrap_err()
            .code,
        ErrorCode::StorageUnavailable
    );
    assert_eq!(service.get(&original.id).unwrap(), original);
    assert_eq!(secrets.len(), 2);
    assert_eq!(count(&database, "credential_journal"), 1);
    drop(service);
    database
        .execute("DROP TRIGGER fail_account_update", [])
        .unwrap();
    let reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    assert_eq!(reopened.get(&original.id).unwrap().revision, 2);
    assert_eq!(secrets.len(), 1);
}

#[test]
fn failed_intent_write_never_writes_secret() {
    let (directory, secrets, mut service) = setup();
    let database = db(directory.path());
    database.execute_batch("CREATE TRIGGER fail_journal BEFORE INSERT ON credential_journal BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert_eq!(
        service
            .create(create_request(&Uuid::new_v4().to_string(), config()))
            .unwrap_err()
            .code,
        ErrorCode::StorageUnavailable
    );
    assert_eq!(secrets.len(), 0);
    assert_eq!(count(&database, "accounts"), 0);
    assert_eq!(count(&database, "credential_journal"), 0);
}

#[test]
fn recovery_cannot_overwrite_newer_revision() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    secrets.fail_next_set(1, true);
    assert!(
        service
            .update_connection(update_request(&original, config()))
            .is_err()
    );
    drop(service);
    db(directory.path())
        .execute("UPDATE accounts SET revision=3,display_name='Newer'", [])
        .unwrap();
    let reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    let saved = reopened.get(&original.id).unwrap();
    assert_eq!(saved.revision, 3);
    assert_eq!(saved.display_name, "Newer");
    assert_eq!(secrets.len(), 1);
}

#[test]
fn missing_secret_is_not_reported_as_configured_or_treated_as_temporary_failure() {
    let (_directory, secrets, mut service) = setup();
    let original = create(&mut service);
    secrets.state.lock().unwrap().entries.clear();
    let missing = service.get(&original.id).unwrap();
    assert!(!missing.authentication.imap.password_configured);
    assert_eq!(
        service
            .update_connection(update_request(&original, keep_config()))
            .unwrap_err()
            .code,
        ErrorCode::SecretMissing
    );
    secrets.state.lock().unwrap().get_error = Some(SecretStoreError::Unavailable);
    assert_eq!(
        service.get(&original.id).unwrap_err().code,
        ErrorCode::SecretStoreUnavailable
    );
}

#[test]
fn database_uses_wal_and_schema_has_composite_foreign_key() {
    let (directory, _secrets, _service) = setup();
    let database = db(directory.path());
    let mode: String = database
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let targets: Vec<(String, String)> = database
        .prepare("PRAGMA foreign_key_list(mail_connections)")
        .unwrap()
        .query_map([], |r| Ok((r.get(3)?, r.get(4)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(targets.contains(&("account_id".into(), "account_id".into())));
    assert!(targets.contains(&("auth_profile_id".into(), "id".into())));
}

#[test]
fn same_process_retry_recovers_create_after_credential_confirmation_failure() {
    let (directory, secrets, mut service) = setup();
    let request_id = Uuid::new_v4().to_string();
    secrets.fail_next_get(1);
    assert_eq!(
        service
            .create(create_request(&request_id, config()))
            .unwrap_err()
            .code,
        ErrorCode::SecretStoreUnavailable
    );
    // Read-only operations expose the last committed snapshot; they do not
    // decide whether an interrupted write should be committed or rolled back.
    assert!(service.list().unwrap().is_empty());
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 1);
    let recovered = service
        .create(create_request(&request_id, config()))
        .unwrap();
    assert_eq!(service.list().unwrap(), vec![recovered]);
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

#[test]
fn same_process_split_recovery_preserves_pending_trust_and_rejects_old_revision() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    let mut split = keep_config();
    split["smtp"]["host"] = json!("other-smtp.example.com");
    split["authentication"] = json!({"kind":"password","mode":"separate","imapPassword":{"action":"keep"},"smtpPassword":{"action":"keep"}});
    // Splitting shared authentication copies the password under a fresh private
    // reference but retains its existing trusted recipients.
    secrets.fail_next_set(1, true);
    assert_eq!(
        service
            .update_connection(update_request(&original, split.clone()))
            .unwrap_err()
            .code,
        ErrorCode::SecretStoreUnavailable
    );
    assert_eq!(service.get(&original.id).unwrap(), original);
    assert_eq!(secrets.len(), 2);
    assert_eq!(
        service
            .update_connection(update_request(&original, split))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let recovered = service.get(&original.id).unwrap();
    assert_eq!(recovered.revision, 2);
    assert_eq!(recovered.mailbox_source_id, original.mailbox_source_id);
    assert_eq!(
        recovered.authentication.imap.trust,
        CredentialTrust::Trusted
    );
    assert_eq!(
        recovered.authentication.smtp.trust,
        CredentialTrust::NeedsConfirmation
    );
    assert_ne!(
        recovered.authentication.imap.id,
        recovered.authentication.smtp.id
    );
    assert_eq!(secrets.len(), 2);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

#[test]
fn separate_merge_split_does_not_transfer_the_other_passwords_trust() {
    let (_directory, secrets, mut service) = setup();
    let mut separate = config();
    separate["authentication"] = json!({"kind":"password","mode":"separate","imapPassword":{"action":"replace","value":"imap-only-secret"},"smtpPassword":{"action":"replace","value":"smtp-only-secret"}});
    let original = service
        .create(create_request(&Uuid::new_v4().to_string(), separate))
        .unwrap();
    let merged = service
        .update_connection(update_request(&original, keep_config()))
        .unwrap();
    assert_eq!(merged.authentication.imap.trust, CredentialTrust::Trusted);
    assert_eq!(
        merged.authentication.smtp.trust,
        CredentialTrust::NeedsConfirmation
    );
    assert_eq!(secrets.len(), 1);
    let mut split = keep_config();
    split["authentication"] = json!({"kind":"password","mode":"separate","imapPassword":{"action":"keep"},"smtpPassword":{"action":"keep"}});
    let split = service
        .update_connection(update_request(&merged, split))
        .unwrap();
    assert_eq!(split.authentication.imap.trust, CredentialTrust::Trusted);
    assert_eq!(
        split.authentication.smtp.trust,
        CredentialTrust::NeedsConfirmation
    );
    assert_eq!(secrets.len(), 2);
}

#[test]
fn committed_create_survives_failed_followup_read_without_duplicate_or_secret_rotation() {
    let (_directory, secrets, mut service) = setup();
    let request_id = Uuid::new_v4().to_string();
    let original = service
        .create(create_request(&request_id, config()))
        .unwrap();
    secrets.fail_next_get(1);
    assert_eq!(
        service.get(&original.id).unwrap_err().code,
        ErrorCode::SecretStoreUnavailable
    );
    let replay = service
        .create(create_request(&request_id, config()))
        .unwrap();
    assert_eq!(replay, original);
    assert_eq!(service.list().unwrap(), vec![original]);
    assert_eq!(secrets.len(), 1);
}

#[test]
fn same_process_partial_update_cleanup_never_deletes_other_accounts_credentials() {
    let (directory, secrets, mut service) = setup();
    let first = create(&mut service);
    let second = create(&mut service);
    let mut separate = config();
    separate["authentication"] = json!({"kind":"password","mode":"separate","imapPassword":{"action":"replace","value":"first-imap-new"},"smtpPassword":{"action":"replace","value":"first-smtp-new"}});
    secrets.fail_next_set(2, false);
    assert_eq!(
        service
            .update_connection(update_request(&first, separate))
            .unwrap_err()
            .code,
        ErrorCode::SecretStoreUnavailable
    );
    assert_eq!(secrets.len(), 3);
    // Any subsequent write resolves the pending intent before taking its base.
    let updated_second = service
        .update_connection(update_request(&second, keep_config()))
        .unwrap();
    assert_eq!(updated_second.revision, 2);
    assert_eq!(service.get(&first.id).unwrap(), first);
    assert_eq!(secrets.len(), 2);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

fn profile_request(account: &Account, display_name: &str, color: Value) -> UpdateProfileRequest {
    serde_json::from_value(
        json!({"accountId":account.id,"profile":{"displayName":display_name,"color":color}}),
    )
    .unwrap()
}

#[test]
fn provider_icon_creation_defaults_and_old_create_identities_remain_compatible() {
    let (directory, _secrets, mut service) = setup();
    let request_id = Uuid::new_v4().to_string();
    let original = service
        .create(create_request(&request_id, config()))
        .unwrap();
    assert!(original.use_provider_icon);
    let saved: String = db(directory.path())
        .query_row(
            "SELECT canonical_config FROM create_requests WHERE request_id=?1",
            [&request_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!saved.contains("useProviderIcon"));
    let mut explicit_default = config();
    explicit_default["useProviderIcon"] = json!(true);
    assert_eq!(
        service
            .create(create_request(&request_id, explicit_default))
            .unwrap(),
        original
    );
    let mut disabled = config();
    disabled["useProviderIcon"] = json!(false);
    assert_eq!(
        service
            .create(create_request(&request_id, disabled.clone()))
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    );
    let disabled_request_id = Uuid::new_v4().to_string();
    let account = service
        .create(create_request(&disabled_request_id, disabled.clone()))
        .unwrap();
    assert!(!account.use_provider_icon);
    assert_eq!(
        service
            .create(create_request(&disabled_request_id, disabled))
            .unwrap(),
        account
    );
    assert_eq!(
        service
            .create(create_request(&disabled_request_id, config()))
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    );

    let mut old_account = serde_json::to_value(&original).unwrap();
    old_account
        .as_object_mut()
        .unwrap()
        .remove("useProviderIcon");
    assert_eq!(
        serde_json::from_value::<Account>(old_account).unwrap(),
        original
    );
}

#[test]
fn provider_icon_profile_is_persisted_isolated_and_kept_after_connection_updates() {
    let (directory, secrets, mut service) = setup();
    let first = create(&mut service);
    let second = create(&mut service);
    let secret_writes = secrets.state.lock().unwrap().set_count;
    let disabled = service
        .update_profile(
            serde_json::from_value(json!({
                "accountId": first.id,
                "profile": {"displayName": first.display_name, "color": first.color,
                    "senderName": first.sender_name, "useProviderIcon": false},
            }))
            .unwrap(),
        )
        .unwrap();
    assert!(!disabled.use_provider_icon);
    assert_eq!(disabled.revision, first.revision);
    assert_eq!(disabled.authentication, first.authentication);
    assert_eq!(disabled.mailbox_source_id, first.mailbox_source_id);
    assert_eq!(disabled.route, first.route);
    assert_eq!(disabled.route_revision, first.route_revision);
    assert_eq!(secrets.state.lock().unwrap().set_count, secret_writes);
    assert_eq!(service.get(&second.id).unwrap(), second);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);

    drop(service);
    let mut reopened = AccountService::open(directory.path(), secrets).unwrap();
    assert!(!reopened.get(&first.id).unwrap().use_provider_icon);
    assert!(reopened.get(&second.id).unwrap().use_provider_icon);
    let reconnected = reopened
        .update_connection(update_request(&disabled, keep_config()))
        .unwrap();
    assert!(!reconnected.use_provider_icon);
    assert_eq!(reconnected.revision, first.revision + 1);
    assert_eq!(reopened.get(&first.id).unwrap(), reconnected);
    assert_eq!(reopened.get(&second.id).unwrap(), second);
}

fn remove_icon_preference_from_pending_journal(database: &Connection) {
    let snapshot: String = database
        .query_row("SELECT snapshot_json FROM credential_journal", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut snapshot: Value = serde_json::from_str(&snapshot).unwrap();
    snapshot["account"]
        .as_object_mut()
        .unwrap()
        .remove("useProviderIcon");
    database
        .execute(
            "UPDATE credential_journal SET snapshot_json=?1",
            [serde_json::to_string(&snapshot).unwrap()],
        )
        .unwrap();
}

#[test]
fn legacy_creation_journal_defaults_provider_icon_to_enabled() {
    let (directory, secrets, mut service) = setup();
    let request_id = Uuid::new_v4().to_string();
    secrets.fail_next_set(1, true);
    assert!(
        service
            .create(create_request(&request_id, config()))
            .is_err()
    );
    remove_icon_preference_from_pending_journal(&db(directory.path()));
    drop(service);

    let mut reopened = AccountService::open(directory.path(), secrets).unwrap();
    let recovered = reopened
        .create(create_request(&request_id, config()))
        .unwrap();
    assert!(recovered.use_provider_icon);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

#[test]
fn legacy_connection_journal_preserves_a_disabled_provider_icon_profile() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    secrets.fail_next_set(1, true);
    assert!(
        service
            .update_connection(update_request(&original, config()))
            .is_err()
    );
    let database = db(directory.path());
    remove_icon_preference_from_pending_journal(&database);
    database
        .execute("UPDATE accounts SET use_provider_icon=0", [])
        .unwrap();
    drop(service);

    let reopened = AccountService::open(directory.path(), secrets).unwrap();
    let recovered = reopened.get(&original.id).unwrap();
    assert!(!recovered.use_provider_icon);
    assert_eq!(recovered.revision, original.revision + 1);
    assert_eq!(count(&database, "credential_journal"), 0);
}

#[test]
fn creates_with_a_normalized_color_and_defaults_to_none() {
    let (_directory, _secrets, mut service) = setup();
    assert_eq!(create(&mut service).color, None);
    let mut value = config();
    value["color"] = json!("  #3B82F6 ");
    let account = service
        .create(create_request(&Uuid::new_v4().to_string(), value))
        .unwrap();
    assert_eq!(account.color.as_deref(), Some("#3b82f6"));
    assert_eq!(
        service.get(&account.id).unwrap().color.as_deref(),
        Some("#3b82f6")
    );
}

#[test]
fn profile_updates_leave_the_connection_revision_and_credentials_alone() {
    let (directory, secrets, mut service) = setup();
    let original = create(&mut service);
    let renamed = service
        .update_profile(profile_request(&original, "  Work ", json!("#16A34A")))
        .unwrap();
    assert_eq!(renamed.display_name, "Work");
    assert_eq!(renamed.color.as_deref(), Some("#16a34a"));
    // The connection version is what sync work is guarded by; it must not move.
    assert_eq!(renamed.revision, original.revision);
    assert_eq!(renamed.mailbox_source_id, original.mailbox_source_id);
    assert_eq!(renamed.authentication, original.authentication);
    assert_eq!(renamed.route_revision, original.route_revision);
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);

    let cleared = service
        .update_profile(profile_request(&renamed, "Work", json!(null)))
        .unwrap();
    assert_eq!(cleared.color, None);
    let emptied = service
        .update_profile(profile_request(&renamed, "Work", json!("")))
        .unwrap();
    assert_eq!(emptied.color, None);

    // A stale connection revision is still the only optimistic check.
    let reconnected = service
        .update_connection(update_request(&original, keep_config()))
        .unwrap();
    assert_eq!(reconnected.revision, original.revision + 1);
}

#[test]
fn connection_updates_keep_the_profile() {
    let (_directory, _secrets, mut service) = setup();
    let original = create(&mut service);
    let profiled = service
        .update_profile(profile_request(&original, "Renamed", json!("#7c3aed")))
        .unwrap();
    let mut moved = keep_config();
    moved["imap"]["port"] = json!(143);
    moved["imap"]["tls"] = json!("startTls");
    let updated = service
        .update_connection(update_request(&profiled, moved))
        .unwrap();
    assert_eq!(updated.display_name, "Renamed");
    assert_eq!(updated.color.as_deref(), Some("#7c3aed"));
    assert_eq!(service.get(&original.id).unwrap(), updated);
}

#[test]
fn profile_updates_validate_before_writing() {
    let (_directory, _secrets, mut service) = setup();
    let original = create(&mut service);
    for color in ["blue", "#12345", "#1234567", "123456", "#gggggg"] {
        assert_eq!(
            service
                .update_profile(profile_request(&original, "Work", json!(color)))
                .unwrap_err(),
            AppError::validation("color"),
            "{color}"
        );
    }
    assert_eq!(
        service
            .update_profile(profile_request(&original, "   ", json!(null)))
            .unwrap_err(),
        AppError::validation("displayName")
    );
    let mut missing = original.clone();
    missing.id = Uuid::new_v4().to_string();
    assert_eq!(
        service
            .update_profile(profile_request(&missing, "Work", json!(null)))
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(service.get(&original.id).unwrap(), original);
    // The profile request accepts nothing else, so it cannot reach a connection.
    assert!(
        serde_json::from_value::<UpdateProfileRequest>(json!({
            "accountId": original.id,
            "profile": {"displayName": "Work", "color": null, "imap": {"host": "evil.example"}},
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<UpdateConnectionRequest>(json!({
            "accountId": original.id, "expectedRevision": original.revision,
            "connection": config(),
        }))
        .is_err(),
        "a connection update must not carry the profile"
    );
}

fn sender_request(account: &Account, sender_name: Value) -> UpdateProfileRequest {
    serde_json::from_value(json!({
        "accountId": account.id,
        "profile": {"displayName": account.display_name, "color": account.color, "senderName": sender_name},
    }))
    .unwrap()
}

#[test]
fn sender_name_is_trimmed_cleared_persisted_and_kept_across_other_updates() {
    let (directory, _secrets, mut service) = setup();
    let original = create(&mut service);
    assert_eq!(original.sender_name, None);
    let stored = |directory: &TempDir| -> Option<String> {
        db(directory.path())
            .query_row("SELECT sender_name FROM accounts", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(stored(&directory), None);

    let named = service
        .update_profile(sender_request(&original, json!("  张三 ")))
        .unwrap();
    assert_eq!(named.sender_name.as_deref(), Some("张三"));
    assert_eq!(stored(&directory).as_deref(), Some("张三"));
    // Like the other profile fields it does not advance the connection revision.
    assert_eq!(named.revision, original.revision);
    assert_eq!(service.get(&original.id).unwrap(), named);

    // A profile request without the field (older callers) clears it.
    let unnamed = service
        .update_profile(profile_request(&named, "Work", json!(null)))
        .unwrap();
    assert_eq!(unnamed.sender_name, None);
    service
        .update_profile(sender_request(&original, json!("Kept")))
        .unwrap();
    // A connection update keeps it, like the display name.
    let moved = service
        .update_connection(update_request(&original, keep_config()))
        .unwrap();
    assert_eq!(moved.sender_name.as_deref(), Some("Kept"));

    for empty in [json!(null), json!(""), json!("  \t ")] {
        let cleared = service
            .update_profile(sender_request(&original, empty))
            .unwrap();
        assert_eq!(cleared.sender_name, None);
        assert_eq!(stored(&directory), None);
    }
    let longest = "n".repeat(128);
    assert_eq!(
        service
            .update_profile(sender_request(&original, json!(longest)))
            .unwrap()
            .sender_name,
        Some(longest)
    );
}

#[test]
fn sender_name_rejects_header_injection_control_characters_and_long_values() {
    let (directory, _secrets, mut service) = setup();
    let original = create(&mut service);
    let named = service
        .update_profile(sender_request(&original, json!("Alice")))
        .unwrap();
    for bad in [
        "Alice\r\nBcc: evil@example.com",
        "Alice\nBcc: x",
        "Alice\rx",
        "tab\there",
        "nul\u{0}",
        "del\u{7f}",
        &"n".repeat(129),
    ] {
        assert_eq!(
            service
                .update_profile(sender_request(&original, json!(bad)))
                .unwrap_err(),
            AppError::validation("senderName"),
            "{bad:?}"
        );
    }
    // Nothing was written, and creation applies the same rules.
    assert_eq!(service.get(&original.id).unwrap(), named);
    let mut value = config();
    value["senderName"] = json!("Bob\r\nX: y");
    assert_eq!(
        service
            .create(create_request(&Uuid::new_v4().to_string(), value))
            .unwrap_err(),
        AppError::validation("senderName")
    );
    assert_eq!(count(&db(directory.path()), "accounts"), 1);

    let mut value = config();
    value["senderName"] = json!("  Carol ");
    let created = service
        .create(create_request(&Uuid::new_v4().to_string(), value))
        .unwrap();
    assert_eq!(created.sender_name.as_deref(), Some("Carol"));
}
