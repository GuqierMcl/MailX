use super::*;

fn ews_config() -> Value {
    let mut value = config();
    let fields = value.as_object_mut().unwrap();
    fields.remove("imap");
    fields.remove("smtp");
    fields.insert("backend".into(), json!("ews"));
    fields.insert(
        "ews".into(),
        json!({
            "url":"https://mail.example.com/EWS/Exchange.asmx",
            "username":"EXAMPLE\\alice"
        }),
    );
    value
}

#[test]
fn ews_accounts_store_only_real_endpoints_and_one_secret() {
    let (directory, secrets, mut service) = setup();
    let request_id = Uuid::new_v4().to_string();
    let account = service
        .create(create_request(&request_id, ews_config()))
        .unwrap();
    assert!(account.connection.is_ews());
    assert_eq!(
        account.authentication.receive().id,
        account.authentication.submit().id
    );
    let database = db(directory.path());
    assert_eq!(count(&database, "mail_connections"), 0);
    assert_eq!(count(&database, "ews_connections"), 1);
    assert_eq!(count(&database, "auth_profiles"), 1);
    assert_eq!(secrets.len(), 1);
    let trust: String = database
        .query_row(
            "SELECT trusted_endpoints_json FROM auth_profiles",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let trust: Value = serde_json::from_str(&trust).unwrap();
    assert_eq!(trust.as_array().unwrap().len(), 1);
    assert_eq!(trust[0]["path"], "/EWS/Exchange.asmx");
    let json = serde_json::to_value(&account).unwrap();
    assert!(json.get("imap").is_none() && json.get("smtp").is_none());
    assert_eq!(json["backend"], "ews");
    assert!(!json.to_string().contains("test-mail-secret"));
    assert_eq!(
        service
            .create(create_request(&request_id, ews_config()))
            .unwrap(),
        account
    );
    drop(service);
    let service = AccountService::open(directory.path(), secrets).unwrap();
    assert_eq!(service.get(&account.id).unwrap(), account);
}

#[test]
fn ews_route_and_password_edits_preserve_backend_and_source() {
    let (_directory, _secrets, mut service) = setup();
    let original = service
        .create(create_request(&Uuid::new_v4().to_string(), ews_config()))
        .unwrap();
    let mut config = ews_config();
    config["authentication"]["password"] = json!({"action":"keep"});
    config["route"] = json!({"kind":"inherit"});
    let routed = service
        .update_connection(update_request(&original, config.clone()))
        .unwrap();
    assert_eq!(routed.connection, original.connection);
    assert_eq!(routed.mailbox_source_id, original.mailbox_source_id);
    assert_eq!(
        routed.authentication.receive().trust,
        CredentialTrust::Trusted
    );
    config["authentication"]["password"] = json!({"action":"replace","value":"new-password"});
    let changed = service
        .update_connection(update_request(&routed, config))
        .unwrap();
    assert_eq!(changed.mailbox_source_id, original.mailbox_source_id);
    assert!(
        changed.authentication.receive().generation > original.authentication.receive().generation
    );
}

#[test]
fn ews_target_changes_require_trust_and_isolate_mailbox_sources() {
    let (_directory, _secrets, mut service) = setup();
    for (field, replacement) in [
        ("url", "https://other.example.com/EWS/Exchange.asmx"),
        ("url", "https://mail.example.com/Other/Exchange.asmx"),
        ("username", "EXAMPLE\\bob"),
    ] {
        let original = service
            .create(create_request(&Uuid::new_v4().to_string(), ews_config()))
            .unwrap();
        let mut config = ews_config();
        config["authentication"]["password"] = json!({"action":"keep"});
        config["ews"][field] = json!(replacement);
        let changed = service
            .update_connection(update_request(&original, config))
            .unwrap();
        assert_ne!(changed.mailbox_source_id, original.mailbox_source_id);
        assert_eq!(
            changed.authentication.receive().trust,
            CredentialTrust::NeedsConfirmation
        );
    }
}

#[test]
fn ews_cannot_silently_replace_an_existing_imap_account() {
    let (_directory, _secrets, mut service) = setup();
    let account = create(&mut service);
    assert_eq!(
        service
            .update_connection(update_request(&account, ews_config()))
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
    assert_eq!(service.get(&account.id).unwrap(), account);
}

#[test]
fn ews_requires_verified_https_and_shared_passwords() {
    let (directory, secrets, mut service) = setup();
    for url in [
        "http://mail.example.com/EWS/Exchange.asmx",
        "https://user:pass@mail.example.com/",
        "https://mail.example.com/?token=x",
        "https://mail.example.com/#secret",
        "https://mail.example.com:0/",
    ] {
        let mut input = ews_config();
        input["ews"]["url"] = json!(url);
        assert_eq!(
            service
                .create(create_request(&Uuid::new_v4().to_string(), input))
                .unwrap_err()
                .code,
            ErrorCode::Validation
        );
    }
    let mut input = ews_config();
    input["authentication"] = json!({"kind":"password","mode":"separate","imapPassword":{"action":"replace","value":"one"},"smtpPassword":{"action":"replace","value":"two"}});
    assert_eq!(
        service
            .create(create_request(&Uuid::new_v4().to_string(), input))
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
    assert_eq!(secrets.len(), 0);
    assert_eq!(count(&db(directory.path()), "accounts"), 0);
}

#[test]
fn protocol_configuration_rejects_ambiguous_or_unknown_branches() {
    let mut input = ews_config();
    input["imap"] = config()["imap"].clone();
    input["smtp"] = config()["smtp"].clone();
    assert!(serde_json::from_value::<AccountConfigInput>(input).is_err());
    let mut input = config();
    input["backend"] = json!("invented");
    assert!(serde_json::from_value::<AccountConfigInput>(input).is_err());
    let mut input = config();
    input["secretRef"] = json!("untrusted-reference");
    assert!(serde_json::from_value::<AccountConfigInput>(input).is_err());
}

#[test]
fn legacy_create_canonical_and_snapshot_wire_shapes_stay_unchanged() {
    let (directory, _secrets, mut service) = setup();
    let request = Uuid::new_v4().to_string();
    let account = service.create(create_request(&request, config())).unwrap();
    let canonical: String = db(directory.path())
        .query_row(
            "SELECT canonical_config FROM create_requests WHERE request_id=?1",
            [&request],
            |r| r.get(0),
        )
        .unwrap();
    let input = config();
    let historical = json!({"displayName":"Personal","emailAddress":"alice@example.com","providerId":null,"imap":input["imap"],"smtp":input["smtp"],"authenticationMode":"shared","route":{"kind":"direct"}});
    assert_eq!(canonical, historical.to_string());
    let value = serde_json::to_value(&account).unwrap();
    assert!(value.get("backend").is_none() && value.get("connection").is_none());
    assert_eq!(serde_json::from_value::<Account>(value).unwrap(), account);
}

#[test]
fn ews_credential_journal_recovers_after_a_lost_secret_write_response() {
    let (directory, secrets, mut service) = setup();
    let request = Uuid::new_v4().to_string();
    secrets.fail_next_set(1, true);
    assert_eq!(
        service
            .create(create_request(&request, ews_config()))
            .unwrap_err()
            .code,
        ErrorCode::SecretStoreUnavailable
    );
    drop(service);
    let mut reopened = AccountService::open(directory.path(), secrets.clone()).unwrap();
    let account = reopened
        .create(create_request(&request, ews_config()))
        .unwrap();
    assert!(account.connection.is_ews());
    assert_eq!(secrets.len(), 1);
    assert_eq!(count(&db(directory.path()), "credential_journal"), 0);
}

#[test]
fn removing_an_ews_account_clears_its_endpoint_and_secret_only() {
    let (directory, secrets, mut service) = setup();
    let ordinary = create(&mut service);
    let exchange = service
        .create(create_request(&Uuid::new_v4().to_string(), ews_config()))
        .unwrap();
    let removed = service
        .remove(mailx_core::RemoveAccountRequest {
            account_id: exchange.id.clone(),
            expected_revision: exchange.revision,
        })
        .unwrap();
    assert!(!removed.credentials_pending);
    assert_eq!(service.get(&ordinary.id).unwrap(), ordinary);
    assert_eq!(count(&db(directory.path()), "ews_connections"), 0);
    assert_eq!(count(&db(directory.path()), "mail_connections"), 2);
    assert_eq!(secrets.len(), 1);
}
