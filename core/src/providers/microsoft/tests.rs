use super::*;
use crate::auth::id_token::fixture_token;
use serde_json::json;
use std::sync::Mutex;

const TENANT: &str = "11111111-2222-3333-4444-555555555555";
const OBJECT: &str = "AAAAAAAA-bbbb-cccc-dddd-eeeeeeeeeeee";

/// Records what the verifier asked for and answers with a fixed result.
struct ScriptedProof {
    result: Result<(), OAuthError>,
    seen: Mutex<Vec<(String, String, u16)>>,
}
impl MailboxProof for ScriptedProof {
    fn prove<'a>(
        &'a self,
        _: &'a NetRoute,
        imap: &'a MailProtocolPolicy,
        mailbox: &'a str,
        _: &'a SecretValue,
    ) -> Pin<Box<dyn Future<Output = Result<(), OAuthError>> + Send + 'a>> {
        self.seen
            .lock()
            .unwrap()
            .push((mailbox.into(), imap.host.clone(), imap.port));
        Box::pin(async move { self.result })
    }
}

fn claims(tid: &str) -> serde_json::Value {
    json!({
        "iss": format!("https://login.microsoftonline.com/{tid}/v2.0"),
        "sub": "pairwise-subject", "aud": CLIENT_ID, "exp": 32_000_000_000u64,
        "nonce": "n-1", "tid": tid, "oid": OBJECT,
        "email": "someone-else@example.com",
    })
}
fn verifier(
    audience: Audience,
    proof: Result<(), OAuthError>,
) -> (MicrosoftVerifier, Arc<ScriptedProof>) {
    let proof = Arc::new(ScriptedProof {
        result: proof,
        seen: Mutex::default(),
    });
    let provider = MicrosoftProvider::new(audience);
    (
        MicrosoftVerifier {
            audience,
            imap: provider.registration.mail_policy.imap.clone(),
            proof: proof.clone(),
        },
        proof,
    )
}
/// The identity check on its own: it never signs in to a mail server.
async fn check(
    audience: Audience,
    claims: serde_json::Value,
) -> (Result<CheckedIdentity, OAuthError>, Arc<ScriptedProof>) {
    let (verifier, proof) = verifier(audience, Ok(()));
    let access = SecretValue::new("access".into());
    let id_token = fixture_token(claims);
    let evidence = IdentityEvidence {
        access_token: &access,
        id_token: Some(&id_token),
        nonce: Some("n-1"),
        client_id: CLIENT_ID,
    };
    let result = verifier.check(&evidence, &NetRoute::default()).await;
    (result, proof)
}

#[test]
fn registrations_are_valid_and_match_the_reviewed_facts() {
    let personal = registration(Audience::Personal);
    let work = registration(Audience::Organization);
    for r in [&personal, &work] {
        r.validate().unwrap();
        assert_eq!(r.client_id, "44a7bc3e-d9fd-4af9-90a6-a03b62013b40");
        assert_eq!(r.redirect_path, "");
        assert_eq!(r.redirect_uri(5000), "http://127.0.0.1:5000");
        assert_eq!(r.resource, None);
        assert!(r.login_hint);
        assert_eq!(
            r.extra_authorize_params,
            [("prompt".to_owned(), "select_account".to_owned())]
        );
        assert_eq!(r.scopes.len(), 6);
        assert!(
            r.scopes
                .contains(&"https://outlook.office.com/SMTP.Send".into())
        );
        assert!(
            r.scopes
                .contains(&"https://outlook.office.com/IMAP.AccessAsUser.All".into())
        );
        let imap = &r.mail_policy.imap;
        assert_eq!(
            (imap.host.as_str(), imap.port),
            ("outlook.office365.com", 993)
        );
        assert_eq!(imap.tls, TlsMode::Implicit);
        assert_eq!(r.mail_policy.smtp.port, 587);
        assert_eq!(r.mail_policy.smtp.tls, TlsMode::StartTls);
        assert_eq!(r.mail_policy.smtp.login, MailLoginPolicy::Mailbox);
    }
    assert_eq!(personal.id, "microsoft-personal");
    assert_eq!(work.id, "microsoft-work");
    assert_ne!(personal.registration_id, work.registration_id);
    assert_eq!(personal.mail_policy.smtp.host, "smtp-mail.outlook.com");
    assert_eq!(work.mail_policy.smtp.host, "smtp.office365.com");
    assert_eq!(
        personal.authorization_endpoint.as_str(),
        "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize"
    );
    assert_eq!(
        personal.token_endpoint.as_str(),
        "https://login.microsoftonline.com/consumers/oauth2/v2.0/token"
    );
    assert_eq!(
        work.authorization_endpoint.as_str(),
        "https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize"
    );
    assert_eq!(
        work.token_endpoint.as_str(),
        "https://login.microsoftonline.com/organizations/oauth2/v2.0/token"
    );
}

#[test]
fn reviewed_servers_fill_the_account_settings() {
    for audience in [Audience::Personal, Audience::Organization] {
        let registration = registration(audience);
        let (imap, smtp) = registration.mail_connections("  Lin@Outlook.com ");
        assert_eq!(
            (imap.host.as_str(), imap.port),
            ("outlook.office365.com", 993)
        );
        assert_eq!(imap.username, "Lin@Outlook.com");
        assert_eq!(smtp.host, registration.mail_policy.smtp.host);
        assert_eq!((smtp.port, smtp.tls), (587, TlsMode::StartTls));
        registration
            .validate_mail_target("Lin@Outlook.com", &imap, &smtp)
            .unwrap();
    }
}

#[test]
fn authorization_url_has_no_path_and_asks_for_account_selection() {
    let (_, url) = crate::auth::OAuthSession::begin(
        registration(Audience::Personal),
        "http://127.0.0.1:5000",
        Some("lin@outlook.com"),
    )
    .unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["redirect_uri"], "http://127.0.0.1:5000");
    assert_eq!(query["prompt"], "select_account");
    assert_eq!(query["login_hint"], "lin@outlook.com");
    assert!(query.contains_key("nonce"));
    assert!(!query.contains_key("resource"));
}

#[tokio::test]
async fn work_account_is_bound_to_tenant_and_object_with_the_claim_address_as_a_hint() {
    let (result, proof) = check(Audience::Organization, claims(TENANT)).await;
    let identity = result.unwrap();
    assert_eq!(
        identity.issuer,
        format!("https://login.microsoftonline.com/{TENANT}/v2.0")
    );
    assert_eq!(
        identity.subject,
        format!("{TENANT}/{}", OBJECT.to_ascii_lowercase())
    );
    assert_eq!(
        identity.mailbox_hint.as_deref(),
        Some("someone-else@example.com")
    );
    // Reading claims never opens a mail connection.
    assert!(proof.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_hint_is_email_then_preferred_username_and_only_when_address_shaped() {
    let hint = |email: Option<&str>, upn: Option<&str>| {
        let mut value = claims(TENANT);
        let object = value.as_object_mut().unwrap();
        object.remove("email");
        if let Some(email) = email {
            object.insert("email".into(), json!(email));
        }
        if let Some(upn) = upn {
            object.insert("preferred_username".into(), json!(upn));
        }
        value
    };
    for (email, upn, expected) in [
        (
            Some("lin@contoso.com"),
            Some("lin@contoso.onmicrosoft.com"),
            Some("lin@contoso.com"),
        ),
        (
            None,
            Some("lin@contoso.onmicrosoft.com"),
            Some("lin@contoso.onmicrosoft.com"),
        ),
        (
            Some("not an address"),
            Some("lin@contoso.com"),
            Some("lin@contoso.com"),
        ),
        (Some("a@b@c"), None, None),
        (None, Some("just-a-name"), None),
        (None, None, None),
    ] {
        let (result, _) = check(Audience::Organization, hint(email, upn)).await;
        assert_eq!(
            result.unwrap().mailbox_hint.as_deref(),
            expected,
            "{email:?} {upn:?}"
        );
    }
}

#[tokio::test]
async fn a_mailbox_is_proven_only_through_the_reviewed_imap_server() {
    let (verifier, proof) = verifier(Audience::Organization, Ok(()));
    let bearer = SecretValue::new("access".into());
    verifier
        .prove_mailbox(&NetRoute::default(), &bearer, "Lin@Outlook.com")
        .await
        .unwrap();
    assert_eq!(
        *proof.seen.lock().unwrap(),
        [(
            "Lin@Outlook.com".into(),
            "outlook.office365.com".into(),
            993
        )]
    );
}

#[tokio::test]
async fn personal_account_needs_the_consumer_tenant() {
    let (result, _) = check(Audience::Personal, claims(CONSUMER_TENANT)).await;
    assert!(result.unwrap().subject.starts_with(CONSUMER_TENANT));
    let (result, _) = check(Audience::Personal, claims(TENANT)).await;
    assert_eq!(result.err(), Some(OAuthError::UseOrganizationEntry));
    let (result, _) = check(Audience::Organization, claims(CONSUMER_TENANT)).await;
    assert_eq!(result.err(), Some(OAuthError::UsePersonalEntry));
}

#[tokio::test]
async fn malformed_identity_claims_are_rejected() {
    let mut other_issuer = claims(TENANT);
    other_issuer["iss"] = json!("https://login.microsoftonline.com/other/v2.0");
    let mut foreign_issuer = claims(TENANT);
    foreign_issuer["iss"] = json!(format!("https://evil.example/{TENANT}/v2.0"));
    let mut v1_issuer = claims(TENANT);
    v1_issuer["iss"] = json!(format!("https://sts.windows.net/{TENANT}/"));
    let mut bad_tid = claims("not-a-guid");
    bad_tid["tid"] = json!("not-a-guid");
    let mut braced_tid = claims(TENANT);
    braced_tid["tid"] = json!(format!("{{{TENANT}}}"));
    let mut no_tid = claims(TENANT);
    no_tid.as_object_mut().unwrap().remove("tid");
    let mut no_oid = claims(TENANT);
    no_oid.as_object_mut().unwrap().remove("oid");
    let mut bad_oid = claims(TENANT);
    bad_oid["oid"] = json!("user@example.com");
    let mut wrong_audience = claims(TENANT);
    wrong_audience["aud"] = json!("someone-else");
    let mut wrong_nonce = claims(TENANT);
    wrong_nonce["nonce"] = json!("n-2");
    for (name, value) in [
        ("other issuer", other_issuer),
        ("foreign issuer", foreign_issuer),
        ("v1 issuer", v1_issuer),
        ("bad tid", bad_tid),
        ("braced tid", braced_tid),
        ("no tid", no_tid),
        ("no oid", no_oid),
        ("bad oid", bad_oid),
        ("audience", wrong_audience),
        ("nonce", wrong_nonce),
    ] {
        let (result, proof) = check(Audience::Organization, value).await;
        assert_eq!(result.err(), Some(OAuthError::InvalidResponse), "{name}");
        assert!(proof.seen.lock().unwrap().is_empty(), "{name}");
    }
}

#[tokio::test]
async fn a_missing_id_token_is_not_identity() {
    let access = SecretValue::new("access".into());
    let verifier = MicrosoftVerifier {
        audience: Audience::Organization,
        imap: registration(Audience::Organization).mail_policy.imap,
        proof: Arc::new(ScriptedProof {
            result: Ok(()),
            seen: Mutex::default(),
        }),
    };
    let evidence = IdentityEvidence {
        access_token: &access,
        id_token: None,
        nonce: Some("n-1"),
        client_id: CLIENT_ID,
    };
    let result = verifier.check(&evidence, &NetRoute::default()).await;
    assert_eq!(result.err(), Some(OAuthError::InvalidResponse));
}

#[tokio::test]
async fn proof_failures_pass_through_unchanged() {
    for error in [
        OAuthError::MailboxRefused,
        OAuthError::Network,
        OAuthError::Proxy,
        OAuthError::Timeout,
    ] {
        let (verifier, _) = verifier(Audience::Organization, Err(error));
        let bearer = SecretValue::new("access".into());
        let result = verifier
            .prove_mailbox(&NetRoute::default(), &bearer, "lin@contoso.com")
            .await;
        assert_eq!(result.err(), Some(error));
    }
}

#[test]
fn only_smtp_535_5_7_139_means_the_administrator_turned_it_off() {
    let provider = MicrosoftProvider::new(Audience::Organization);
    let smtp = |code, status| MailRejection {
        protocol: MailProtocol::Smtp,
        smtp_code: code,
        enhanced_status: status,
        imap_code: None,
    };
    assert_eq!(
        provider.classify_rejection(&smtp(Some(535), Some((5, 7, 139)))),
        Some(RejectionKind::ProtocolDisabledByAdministrator)
    );
    for rejection in [
        smtp(Some(535), Some((5, 7, 3))),
        smtp(Some(535), Some((5, 7, 8))),
        smtp(Some(535), None),
        smtp(Some(554), Some((5, 7, 139))),
        smtp(None, Some((5, 7, 139))),
        MailRejection {
            protocol: MailProtocol::Imap,
            smtp_code: Some(535),
            enhanced_status: Some((5, 7, 139)),
            imap_code: None,
        },
        MailRejection {
            protocol: MailProtocol::Imap,
            smtp_code: None,
            enhanced_status: None,
            imap_code: Some("AUTHENTICATIONFAILED"),
        },
    ] {
        assert_eq!(
            provider.classify_rejection(&rejection),
            None,
            "{rejection:?}"
        );
    }
}

#[test]
fn guids_must_use_the_canonical_form() {
    assert!(is_guid(TENANT));
    assert!(is_guid(OBJECT));
    for text in [
        "",
        "abc",
        "11111111222233334444555555555555",
        "{11111111-2222-3333-4444-555555555555}",
        "urn:uuid:11111111-2222-3333-4444-555555555555",
    ] {
        assert!(!is_guid(text), "{text}");
    }
}
