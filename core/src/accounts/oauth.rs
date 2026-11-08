//! Backend-only consumption of verified OAuth results. No caller-provided subject
//! or token is accepted by the public account configuration contract.
use super::*;
use crate::auth::credential::ProtocolCredential;
use crate::auth::{
    GrantBinding, ProviderRegistration, TokenManager, TokenSet, VerifiedAuthorization,
};

pub(crate) fn summary(
    binding: &GrantBinding,
    imap: &AuthRecord,
    smtp: &AuthRecord,
    account: &Account,
) -> AccountAuthenticationSummary {
    AccountAuthenticationSummary::OAuth2(OAuthAuthenticationSummary {
        configuration: OAuth2Configuration {
            provider_id: binding.provider_id.clone(),
            issuer: binding.issuer.clone(),
            client_registration_id: binding.client_registration_id.clone(),
            authorization_subject: Some(binding.subject.clone()),
            target_mailbox_resource: Some(binding.mailbox.clone()),
        },
        state: OAuthAuthenticationState::Connected,
        imap: ProtocolAuthenticationSummary::from(
            &imap.summary(&TrustEndpoint::mail("imap", &account.imap)),
        ),
        smtp: ProtocolAuthenticationSummary::from(
            &smtp.summary(&TrustEndpoint::mail("smtp", &account.smtp)),
        ),
    })
}
fn binding_matches_registration(
    binding: &GrantBinding,
    registration: &ProviderRegistration,
) -> bool {
    binding.provider_id == registration.id
        && binding.issuer == registration.issuer
        && binding.client_id == registration.client_id
        && binding.client_registration_id == registration.registration_id
        && binding.resource == registration.resource
        && binding
            .scopes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            == registration
                .scopes
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
}
fn registration_matches_policy(
    registration: &ProviderRegistration,
    policy: &ProviderRegistration,
) -> bool {
    registration.id == policy.id
        && registration.client_id == policy.client_id
        && registration.registration_id == policy.registration_id
        && registration.mail_policy == policy.mail_policy
        && registration.issuer == policy.issuer
        && registration.authorization_endpoint == policy.authorization_endpoint
        && registration.token_endpoint == policy.token_endpoint
        && registration
            .scopes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            == policy
                .scopes
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
        && registration.resource == policy.resource
        && registration
            .allowed_targets
            .iter()
            .collect::<std::collections::HashSet<_>>()
            == policy
                .allowed_targets
                .iter()
                .collect::<std::collections::HashSet<_>>()
}
#[cfg(test)]
pub(super) struct TestProviderPolicy {
    id: String,
    client_id: String,
    registration_id: String,
    issuer: String,
    authorization_endpoint: url::Url,
    token_endpoint: url::Url,
    scopes: Vec<String>,
    resource: Option<String>,
    allowed_targets: Vec<crate::auth::TokenTarget>,
    mail_policy: crate::auth::MailEndpointPolicy,
}
#[cfg(test)]
impl From<&ProviderRegistration> for TestProviderPolicy {
    fn from(value: &ProviderRegistration) -> Self {
        Self {
            id: value.id.clone(),
            client_id: value.client_id.clone(),
            registration_id: value.registration_id.clone(),
            issuer: value.issuer.clone(),
            authorization_endpoint: value.authorization_endpoint.clone(),
            token_endpoint: value.token_endpoint.clone(),
            scopes: value.scopes.clone(),
            resource: value.resource.clone(),
            allowed_targets: value.allowed_targets.clone(),
            mail_policy: value.mail_policy.clone(),
        }
    }
}
#[cfg(test)]
impl TestProviderPolicy {
    fn matches(&self, value: &ProviderRegistration) -> bool {
        self.id == value.id
            && self.mail_policy == value.mail_policy
            && self.client_id == value.client_id
            && self.registration_id == value.registration_id
            && self.issuer == value.issuer
            && self.authorization_endpoint == value.authorization_endpoint
            && self.token_endpoint == value.token_endpoint
            && self
                .scopes
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                == value
                    .scopes
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
            && self.resource == value.resource
            && self
                .allowed_targets
                .iter()
                .collect::<std::collections::HashSet<_>>()
                == value
                    .allowed_targets
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
    }
}
impl AccountService {
    pub(crate) fn oauth_managers_for_shutdown(&mut self) -> Vec<Arc<TokenManager>> {
        self.oauth_shutdown = true;
        let mut all = self
            .oauth_managers
            .lock()
            .map(|managers| managers.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        if let Ok(retired) = self.retired_oauth_managers.lock() {
            all.extend(retired.iter().map(|(_, manager)| manager.clone()));
        }
        for manager in &all {
            // A poisoned admission mutex also rejects every new resolution.
            let _ = manager.close_admission();
        }
        all
    }
    fn load_oauth_registration(&self, id: &str) -> Result<ProviderRegistration, AppError> {
        let text: String = self.storage.db.query_row(
            "SELECT configuration_json FROM oauth_client_registrations WHERE id=?1",
            [id],
            |row| row.get(0),
        )?;
        serde_json::from_str(&text).map_err(|_| AppError::new(ErrorCode::StorageIncompatible))
    }
    #[cfg(test)]
    pub(crate) fn trust_test_oauth_policy(&mut self, registration: &ProviderRegistration) {
        self.trusted_oauth_test_providers.insert(
            registration.id.clone(),
            TestProviderPolicy::from(registration),
        );
    }
    pub(super) fn oauth_cleanup_pending(&self, key: &str) -> bool {
        self.retired_oauth_managers
            .lock()
            .map(|retired| {
                retired
                    .iter()
                    .any(|(retired_key, manager)| retired_key == key && !manager.is_drained())
            })
            .unwrap_or(true)
    }
    /// Keep an invalidated manager until its owned refresh/persistence work
    /// drains. Earlier retirements that already drained are dropped first, so
    /// the list stays bounded and their in-memory grants are released.
    fn retire_oauth_manager(&self, key: String, manager: Arc<TokenManager>) {
        manager.invalidate();
        if let Ok(mut retired) = self.retired_oauth_managers.lock() {
            // Invalidated managers never start new secret work, so one that is
            // drained now stays drained.
            retired.retain(|(_, previous)| !previous.is_drained());
            retired.push((key, manager));
        }
    }
    pub(super) fn invalidate_oauth_refs(&self, keys: &[String]) {
        if let Ok(mut managers) = self.oauth_managers.lock() {
            for key in keys {
                if let Some(manager) = managers.remove(key) {
                    self.retire_oauth_manager(key.clone(), manager);
                }
            }
        }
    }
    pub(super) fn invalidate_account_oauth(&self, account_id: &str) {
        if let Ok(mut managers) = self.oauth_managers.lock() {
            let prefix = format!("mailx/{account_id}/");
            let keys: Vec<String> = managers
                .keys()
                .filter(|key| key.starts_with(&prefix))
                .cloned()
                .collect();
            for key in keys {
                if let Some(manager) = managers.remove(&key) {
                    self.retire_oauth_manager(key, manager);
                }
            }
        }
    }
    pub(super) fn protocol_credential(
        &self,
        profile: &AuthRecord,
        account: &Account,
    ) -> Result<ProtocolCredential, sync_plan::PlanError> {
        if self.oauth_shutdown {
            return Err(sync_plan::PlanError::CredentialMissing);
        }
        if profile.oauth.is_none() {
            return self
                .secrets
                .get(&profile.secret_ref)?
                .map(ProtocolCredential::Password)
                .ok_or(sync_plan::PlanError::CredentialMissing);
        }
        let mut managers = self
            .oauth_managers
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        let binding = profile
            .oauth
            .as_ref()
            .ok_or(sync_plan::PlanError::CredentialMissing)?;
        let registration = if let Some(registration) = self
            .oauth_registrations
            .get(&binding.client_registration_id)
        {
            registration.clone()
        } else {
            let registration = self.load_oauth_registration(&binding.client_registration_id)?;
            let trusted = crate::auth::lookup_registration(&binding.provider_id)
                .ok()
                .is_some_and(|policy| registration_matches_policy(&registration, &policy));
            #[cfg(test)]
            let trusted = trusted
                || self
                    .trusted_oauth_test_providers
                    .get(&registration.id)
                    .is_some_and(|policy| policy.matches(&registration));
            if !trusted
                || registration.id != binding.provider_id
                || registration.issuer != binding.issuer
            {
                return Err(sync_plan::PlanError::CredentialUntrusted);
            }
            registration
        };
        if !binding_matches_registration(binding, &registration) {
            return Err(sync_plan::PlanError::CredentialUntrusted);
        }
        registration
            .validate_mail_target(&binding.mailbox, &account.imap, &account.smtp)
            .map_err(|_| sync_plan::PlanError::CredentialUntrusted)?;
        if let Some(manager) = managers.get(&profile.secret_ref) {
            return Ok(ProtocolCredential::ManagedOAuth2(manager.clone()));
        }
        let stored = self
            .secrets
            .get(&profile.secret_ref)?
            .ok_or(sync_plan::PlanError::CredentialMissing)?;
        let tokens = TokenSet::decode_secret(&stored)
            .map_err(|_| AppError::new(ErrorCode::SecretMissing))?;
        let mut manager = TokenManager::new(
            registration,
            self.secrets.clone(),
            profile.secret_ref.clone(),
        )
        .map_err(|_| AppError::new(ErrorCode::Unsupported))?;
        manager
            .restore(tokens)
            .map_err(|_| AppError::new(ErrorCode::SecretMissing))?;
        let manager = Arc::new(manager);
        managers.insert(profile.secret_ref.clone(), manager.clone());
        Ok(ProtocolCredential::ManagedOAuth2(manager))
    }
    pub(crate) fn prepare_oauth_reauthorization(
        &self,
        account_id: &str,
        expected_revision: u32,
    ) -> Result<(CreateAccountRequest, crate::net::NetRoute), AppError> {
        validate_uuid(account_id, "accountId")?;
        let account = self.storage.load(account_id)?.account;
        if account.revision != expected_revision {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let AccountAuthenticationSummary::OAuth2(auth) = account.authentication else {
            return Err(AppError::new(ErrorCode::Unsupported));
        };
        let mut route = self
            .image_route(account_id)
            .map_err(|_| AppError::new(ErrorCode::SecretMissing))?;
        route.image_proxy_dns = false;
        Ok((
            CreateAccountRequest {
                request_id: new_id(),
                config: AccountConfigInput {
                    display_name: account.display_name,
                    color: account.color,
                    sender_name: account.sender_name,
                    use_provider_icon: account.use_provider_icon,
                    email_address: account.email_address,
                    provider_id: account.provider_id,
                    imap: account.imap,
                    smtp: account.smtp,
                    authentication: AuthenticationInput::OAuth2 {
                        configuration: auth.configuration,
                    },
                    route: RouteInput::Direct,
                },
            },
            route,
        ))
    }
    pub(crate) fn reauthorize_oauth(
        &mut self,
        account_id: &str,
        expected_revision: u32,
        authorization: VerifiedAuthorization,
        registration: ProviderRegistration,
    ) -> Result<Account, AppError> {
        if self.oauth_shutdown {
            return Err(AppError::new(ErrorCode::CoreNotReady));
        }
        validate_uuid(account_id, "accountId")?;
        self.recover()?;
        let old = self.storage.load(account_id)?;
        if old.account.revision != expected_revision {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        registration
            .validate_mail_target(
                &old.account.email_address,
                &old.account.imap,
                &old.account.smtp,
            )
            .map_err(|_| AppError::validation("authentication"))?;
        let old_binding = old
            .profiles
            .iter()
            .find_map(|profile| profile.oauth.as_ref())
            .ok_or_else(|| AppError::new(ErrorCode::Unsupported))?;
        let mut binding = authorization.binding;
        if !binding_matches_registration(&binding, &registration)
            || !binding_matches_registration(old_binding, &registration)
            || binding.subject != old_binding.subject
            || binding.issuer != old_binding.issuer
            || binding.provider_id != old_binding.provider_id
            || binding.client_registration_id != old_binding.client_registration_id
            || !binding
                .mailbox
                .eq_ignore_ascii_case(&old.account.email_address)
            || registration.registration_id != binding.client_registration_id
        {
            return Err(AppError::validation("authentication"));
        }
        binding.generation = old_binding
            .generation
            .checked_add(1)
            .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
        let mut snapshot = old.clone();
        let mut writes = Vec::new();
        for profile in &mut snapshot.profiles {
            if profile.oauth.is_some() {
                profile.generation = next_version(profile.generation)?;
                profile.secret_ref = secret_key(account_id);
                profile.oauth = Some(binding.clone());
                writes.push((
                    profile.secret_ref.clone(),
                    authorization
                        .tokens
                        .encode_secret()
                        .map_err(|_| AppError::new(ErrorCode::Internal))?,
                ));
            }
        }
        snapshot.account.revision = next_version(expected_revision)?;
        snapshot.account.updated_at = now();
        let imap = snapshot
            .profile(&old.account.authentication.imap().id)
            .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
        let smtp = snapshot
            .profile(&old.account.authentication.smtp().id)
            .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
        snapshot.account.authentication = summary(&binding, imap, smtp, &snapshot.account);
        let retained = snapshot.refs();
        let journal = Journal {
            id: new_id(),
            expected_revision: Some(expected_revision),
            new_refs: writes.iter().map(|(key, _)| key.clone()).collect(),
            retired_refs: old
                .refs()
                .into_iter()
                .filter(|key| !retained.contains(key))
                .collect(),
            snapshot,
            create_request: None,
            applied: false,
        };
        let account = self.commit(journal, writes)?;
        self.oauth_registrations
            .insert(registration.registration_id.clone(), registration);
        Ok(account)
    }
    /// The value is produced only by a completed, identity-verified session.
    /// Moving it here consumes the authorization result exactly once.
    pub(crate) fn create_oauth(
        &mut self,
        mut request: CreateAccountRequest,
        authorization: VerifiedAuthorization,
        registration: ProviderRegistration,
    ) -> Result<Account, AppError> {
        if self.oauth_shutdown {
            return Err(AppError::new(ErrorCode::CoreNotReady));
        }
        normalize_oauth_request(&mut request)?;
        validate_uuid(&request.request_id, "requestId")?;
        let binding = authorization.binding;
        registration
            .validate_mail_target(&binding.mailbox, &request.config.imap, &request.config.smtp)
            .map_err(|_| AppError::validation("authentication"))?;
        if !binding_matches_registration(&binding, &registration)
            || !binding
                .mailbox
                .eq_ignore_ascii_case(&request.config.email_address)
            || registration.registration_id != binding.client_registration_id
            || registration.id != binding.provider_id
        {
            return Err(AppError::validation("authentication"));
        }
        let (mut profile, mut connection) = request.config.split();
        validate_profile(&mut profile)?;
        validate_connection(&mut connection)?;
        if !matches!(
            connection.authentication,
            AuthenticationInput::OAuth2 { .. }
        ) {
            return Err(AppError::validation("authentication"));
        }
        if self.storage.create_request(&request.request_id)?.is_some()
            || self
                .storage
                .create_request_was_removed(&request.request_id)?
        {
            return Err(AppError::new(ErrorCode::IdempotencyConflict));
        }
        self.recover()?;
        let id = new_id();
        let secret_ref = secret_key(&id);
        let auth = AuthRecord {
            id: new_id(),
            generation: 1,
            secret_ref: secret_ref.clone(),
            trusted_endpoints: vec![
                TrustEndpoint::mail("imap", &connection.imap),
                TrustEndpoint::mail("smtp", &connection.smtp),
            ],
            oauth: Some(binding.clone()),
        };
        let mut writes = vec![(
            secret_ref,
            authorization
                .tokens
                .encode_secret()
                .map_err(|_| AppError::new(ErrorCode::Internal))?,
        )];
        let (route, proxy_secret) = self.build_route(&id, connection.route, None, &mut writes)?;
        let timestamp = now();
        let mut account = Account {
            id,
            revision: 1,
            display_name: profile.display_name,
            color: profile.color,
            sender_name: profile.sender_name,
            use_provider_icon: profile.use_provider_icon,
            email_address: connection.email_address,
            provider_id: connection.provider_id,
            mailbox_source_id: new_id(),
            imap: connection.imap,
            smtp: connection.smtp,
            authentication: AccountAuthenticationSummary::Password(PasswordAuthenticationSummary {
                mode: AuthenticationMode::Shared,
                imap: auth.summary(&auth.trusted_endpoints[0]),
                smtp: auth.summary(&auth.trusted_endpoints[1]),
            }),
            default_route_revision: matches!(route, RouteConfig::Inherit).then_some(1),
            route,
            route_revision: 1,
            connection_status: ConnectionStatus::Unverified,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        };
        account.authentication = summary(&binding, &auth, &auth, &account);
        let journal = Journal {
            id: new_id(),
            expected_revision: None,
            new_refs: writes.iter().map(|(key, _)| key.clone()).collect(),
            retired_refs: vec![],
            snapshot: Snapshot {
                account,
                profiles: vec![auth],
                proxy_secret,
            },
            create_request: Some((request.request_id, format!("oauth:{}", binding.grant_id))),
            applied: false,
        };
        self.storage.db.execute("INSERT INTO oauth_client_registrations(id,provider_id,configuration_json) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET configuration_json=excluded.configuration_json WHERE provider_id=excluded.provider_id", rusqlite::params![registration.registration_id, registration.id, crate::storage::encode(&registration)?])?;
        let account = self.commit(journal, writes)?;
        self.oauth_registrations
            .insert(registration.registration_id.clone(), registration);
        Ok(account)
    }
}

pub(crate) fn normalize_oauth_request(request: &mut CreateAccountRequest) -> Result<(), AppError> {
    let mut connection = request.config.clone().split().1;
    validate_connection(&mut connection)?;
    request.config.email_address = connection.email_address;
    request.config.imap = connection.imap;
    request.config.smtp = connection.smtp;
    request.config.route = connection.route;
    Ok(())
}

/// Resolve a temporary authorization route without saving proxy/mail secrets.
pub(crate) fn authorization_route(
    request: &CreateAccountRequest,
) -> Result<crate::net::NetRoute, AppError> {
    let mut config = request.config.clone().split().1;
    validate_connection(&mut config)?;
    match config.route {
        RouteInput::Direct | RouteInput::Inherit => Ok(crate::net::NetRoute::default()),
        RouteInput::Custom {
            proxy_kind,
            host,
            port,
            authentication,
            ..
        } => {
            let auth = authentication
                .map(|auth| match auth.password {
                    SecretUpdate::Replace { value } => Ok((auth.username, value)),
                    SecretUpdate::Keep => Err(AppError::validation("route.authentication")),
                })
                .transpose()?;
            Ok(crate::net::NetRoute {
                image_proxy_dns: false,
                proxy: Some(crate::net::ProxyRoute {
                    kind: proxy_kind,
                    host,
                    port,
                    auth,
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::test_support::MemorySecrets;
    use serde_json::json;
    use tempfile::TempDir;

    fn registration() -> ProviderRegistration {
        ProviderRegistration {
            id: "fixture".into(),
            registration_id: "fixture-app".into(),
            issuer: "https://fixture.invalid".into(),
            client_id: "public-client".into(),
            authorization_endpoint: "https://fixture.invalid/authorize".parse().unwrap(),
            token_endpoint: "https://fixture.invalid/token".parse().unwrap(),
            scopes: vec!["mail".into()],
            allowed_targets: vec![],
            mail_policy: crate::auth::MailEndpointPolicy {
                imap: crate::auth::MailProtocolPolicy {
                    host: "imap.example.com".into(),
                    port: 993,
                    tls: TlsMode::Implicit,
                    login: crate::auth::MailLoginPolicy::Fixed("lin".into()),
                },
                smtp: crate::auth::MailProtocolPolicy {
                    host: "smtp.example.com".into(),
                    port: 465,
                    tls: TlsMode::Implicit,
                    login: crate::auth::MailLoginPolicy::Fixed("lin".into()),
                },
            },
            resource: Some("urn:mail".into()),
        }
    }
    fn verified(subject: &str) -> VerifiedAuthorization {
        VerifiedAuthorization {
            binding: GrantBinding {
                provider_id: "fixture".into(),
                issuer: "https://fixture.invalid".into(),
                client_registration_id: "fixture-app".into(),
                client_id: "public-client".into(),
                subject: subject.into(),
                mailbox: "lin@example.com".into(),
                grant_id: new_id(),
                generation: 1,
                resource: Some("urn:mail".into()),
                scopes: vec!["mail".into()],
            },
            tokens: TokenSet {
                access_token: SecretValue::new("opaque-access-never-public".into()),
                refresh_token: Some(SecretValue::new("opaque-refresh-never-public".into())),
                expires_at: u64::MAX,
                target: None,
                scopes: vec!["mail".into()],
            },
        }
    }
    fn request() -> CreateAccountRequest {
        let mut config = super::super::sync_plan::tests::config(json!({"kind":"direct"}));
        config["authentication"] = json!({"kind":"oauth2","configuration":{"providerId":"untrusted-front-end","issuer":"https://attacker.invalid","clientRegistrationId":"attacker","authorizationSubject":"attacker-subject","targetMailboxResource":null}});
        serde_json::from_value(json!({"requestId":new_id(),"config":config})).unwrap()
    }
    fn fixture() -> (TempDir, AccountService, Account, Arc<MemorySecrets>) {
        let dir = TempDir::new().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let mut service = AccountService::open(dir.path(), secrets.clone()).unwrap();
        let account = service
            .create_oauth(request(), verified("stable-subject"), registration())
            .unwrap();
        (dir, service, account, secrets)
    }
    #[test]
    fn verified_identity_overrides_untrusted_configuration_and_public_output_has_no_secrets() {
        let (_dir, service, account, _) = fixture();
        let AccountAuthenticationSummary::OAuth2(auth) =
            service.get(&account.id).unwrap().authentication
        else {
            panic!("OAuth summary expected")
        };
        assert_eq!(
            auth.configuration.authorization_subject.as_deref(),
            Some("stable-subject")
        );
        assert_eq!(auth.configuration.provider_id, "fixture");
        let output = serde_json::to_string(&account).unwrap();
        assert!(!output.contains("opaque-"));
        assert!(!output.contains("secretRef"));
        assert!(!output.contains("passwordConfigured"));
    }
    #[test]
    fn password_keep_cannot_reuse_an_oauth_secret_in_shared_or_separate_mode() {
        let (_dir, mut service, account, _) = fixture();
        for auth in [
            json!({"kind":"password","mode":"shared","password":{"action":"keep"}}),
            json!({"kind":"password","mode":"separate","imapPassword":{"action":"keep"},"smtpPassword":{"action":"replace","value":"fresh-password"}}),
        ] {
            let mut config = super::super::sync_plan::tests::config(json!({"kind":"direct"}));
            config["authentication"] = auth;
            let request = super::super::sync_plan::tests::connection_update(
                &account.id,
                account.revision,
                config,
            );
            assert_eq!(
                service.update_connection(request).unwrap_err().code,
                ErrorCode::Validation
            );
            assert!(service.get(&account.id).unwrap().authentication.is_oauth());
        }
    }
    #[test]
    fn reauthorization_checks_subject_and_revision_then_retires_old_manager() {
        let (_dir, mut service, account, _) = fixture();
        let plan = service.sync_plan(&account.id).unwrap();
        let ProtocolCredential::ManagedOAuth2(manager) = plan.login.credential else {
            panic!("managed credential expected")
        };
        assert_eq!(
            service
                .reauthorize_oauth(&account.id, 1, verified("wrong-subject"), registration())
                .unwrap_err()
                .code,
            ErrorCode::Validation
        );
        assert!(manager.is_valid());
        let changed = service
            .reauthorize_oauth(&account.id, 1, verified("stable-subject"), registration())
            .unwrap();
        assert_eq!(changed.revision, 2);
        assert_eq!(changed.authentication.imap().generation, 2);
        assert!(!manager.is_valid());
        assert_eq!(
            service
                .reauthorize_oauth(&account.id, 1, verified("stable-subject"), registration())
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
    }
    #[test]
    fn removal_invalidates_issued_plans_and_missing_secret_is_reauthorization() {
        let (_dir, mut service, account, secrets) = fixture();
        let snapshot = service.storage.load(&account.id).unwrap();
        secrets.delete(&snapshot.profiles[0].secret_ref).unwrap();
        let AccountAuthenticationSummary::OAuth2(auth) =
            service.get(&account.id).unwrap().authentication
        else {
            panic!()
        };
        assert_eq!(auth.state, OAuthAuthenticationState::NeedsReauthorization);
        secrets
            .set(
                &snapshot.profiles[0].secret_ref,
                &verified("stable-subject").tokens.encode_secret().unwrap(),
            )
            .unwrap();
        let plan = service.sync_plan(&account.id).unwrap();
        let ProtocolCredential::ManagedOAuth2(manager) = plan.login.credential else {
            panic!()
        };
        service
            .remove(RemoveAccountRequest {
                account_id: account.id,
                expected_revision: 1,
            })
            .unwrap();
        assert!(!manager.is_valid());
        assert!(
            secrets
                .get(&snapshot.profiles[0].secret_ref)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn legacy_password_snapshot_without_discriminator_or_oauth_field_still_decodes() {
        let dir = TempDir::new().unwrap();
        let mut service =
            AccountService::open(dir.path(), Arc::new(MemorySecrets::default())).unwrap();
        let request: CreateAccountRequest = serde_json::from_value(json!({"requestId":new_id(),"config":super::super::sync_plan::tests::config(json!({"kind":"direct"}))})).unwrap();
        let account = service.create(request).unwrap();
        let snapshot = service.storage.load(&account.id).unwrap();
        let mut value = serde_json::to_value(snapshot).unwrap();
        value["account"]["authentication"]
            .as_object_mut()
            .unwrap()
            .remove("kind");
        for profile in value["profiles"].as_array_mut().unwrap() {
            profile.as_object_mut().unwrap().remove("oauth");
        }
        let restored: Snapshot = serde_json::from_value(value).unwrap();
        assert!(!restored.account.authentication.is_oauth());
        assert!(
            restored
                .profiles
                .iter()
                .all(|profile| profile.oauth.is_none())
        );
    }
    #[test]
    fn altered_persisted_provider_endpoints_and_client_id_are_rejected_before_network() {
        for field in [
            "token_endpoint",
            "authorization_endpoint",
            "client_id",
            "scopes",
            "resource",
            "allowed_targets",
        ] {
            let (dir, service, account, secrets) = fixture();
            let mut value = serde_json::to_value(registration()).unwrap();
            value[field] = match field {
                "scopes" => json!(["other"]),
                "allowed_targets" => {
                    json!([{ "resource": "https://attacker.invalid", "scopes": ["other"] }])
                }
                _ => json!("https://attacker.invalid/token"),
            };
            service
                .storage
                .db
                .execute(
                    "UPDATE oauth_client_registrations SET configuration_json=?1",
                    [value.to_string()],
                )
                .unwrap();
            drop(service);
            let mut reopened = AccountService::open(dir.path(), secrets).unwrap();
            reopened.trust_test_oauth_policy(&registration());
            assert!(
                reopened.sync_plan(&account.id).is_err(),
                "{field} must fail closed"
            );
        }
    }
    fn queued(
        service: &mut AccountService,
        account: &Account,
        now: chrono::DateTime<chrono::Utc>,
    ) -> crate::send::types::OutboxItem {
        let draft = service.create_draft(serde_json::from_value(json!({"accountId":account.id,"to":[{"name":null,"address":"to@example.org"}],"cc":[],"bcc":[],"subject":"test","bodyHtml":"hello","clientMutationId":new_id()})).unwrap(), "", crate::drafts::types::DraftOrigin::User).unwrap();
        service
            .queue_send(
                &crate::send::types::QueueSendRequest {
                    draft_id: draft.id,
                    expected_revision: draft.revision,
                    client_mutation_id: new_id(),
                },
                0,
                now,
            )
            .unwrap()
            .0
    }
    #[test]
    fn send_admission_is_skipped_when_idle_and_reports_unrecoverable_grants_as_required() {
        use crate::accounts::SendAdmission;
        let (_dir, mut service, account, secrets) = fixture();
        let now = chrono::Utc::now();
        // Nothing due: no snapshot load, plan or token work.
        assert!(matches!(
            service.oauth_send_admission(&account.id, now).unwrap(),
            SendAdmission::NotRequired
        ));
        queued(&mut service, &account, now);
        assert!(matches!(
            service.oauth_send_admission(&account.id, now).unwrap(),
            SendAdmission::Ready { revision: 1, .. }
        ));
        // A missing grant never recovers by itself: ask for reauthorization
        // instead of promising an automatic resume.
        let key = service.storage.load(&account.id).unwrap().profiles[0]
            .secret_ref
            .clone();
        service.invalidate_oauth_refs(std::slice::from_ref(&key));
        secrets.delete(&key).unwrap();
        assert!(matches!(
            service.oauth_send_admission(&account.id, now).unwrap(),
            SendAdmission::Blocked("authorizationRequired")
        ));
        secrets
            .set(&key, &SecretValue::new("not a token set".into()))
            .unwrap();
        assert!(matches!(
            service.oauth_send_admission(&account.id, now).unwrap(),
            SendAdmission::Blocked("authorizationRequired")
        ));
        service.oauth_managers_for_shutdown();
        assert!(matches!(
            service.oauth_send_admission(&account.id, now).unwrap(),
            SendAdmission::Paused
        ));
    }
    #[test]
    fn retired_managers_are_released_once_drained() {
        let (_dir, service, account, _) = fixture();
        let key = service.storage.load(&account.id).unwrap().profiles[0]
            .secret_ref
            .clone();
        for _ in 0..5 {
            service.sync_plan(&account.id).unwrap();
            service.invalidate_oauth_refs(std::slice::from_ref(&key));
        }
        let retired = service.retired_oauth_managers.lock().unwrap();
        assert_eq!(retired.len(), 1);
        assert!(!retired[0].1.is_valid());
        assert!(retired[0].1.cached_token().is_none());
    }
    #[test]
    fn authorization_wait_keeps_queue_and_attempts_and_pauses_an_existing_retry_window() {
        use crate::send::types::{OutboxError, OutboxState};
        let (_dir, mut service, account, _) = fixture();
        let start = chrono::Utc::now();
        let item = queued(&mut service, &account, start);
        service
            .record_oauth_send_blocked(&account.id, "authorizationRequired", start)
            .unwrap();
        let blocked = service.get_outbox(&item.id).unwrap();
        assert_eq!(blocked.state, OutboxState::Queued);
        assert_eq!(blocked.attempts, 0);
        assert_eq!(blocked.error.unwrap().reason, "authorizationRequired");
        let resume = start + chrono::Duration::days(3);
        service
            .clear_oauth_send_blocked(&account.id, resume)
            .unwrap();
        let super::super::Next::Job { job, .. } =
            service.outbox_step(&account.id, resume).unwrap().next
        else {
            panic!()
        };
        assert!((job.retry_started_at - resume).num_milliseconds().abs() < 2);
        service
            .outbox_requeue(&item.id, resume, &OutboxError::reason("connect"), resume)
            .unwrap();
        service
            .record_oauth_send_blocked(&account.id, "authorizationRequired", resume)
            .unwrap();
        let next_resume = resume + chrono::Duration::days(4);
        service
            .clear_oauth_send_blocked(&account.id, next_resume)
            .unwrap();
        let super::super::Next::Job { job, .. } =
            service.outbox_step(&account.id, next_resume).unwrap().next
        else {
            panic!()
        };
        assert_eq!(job.item.attempts, 2);
        assert!(
            (job.retry_started_at - next_resume)
                .num_milliseconds()
                .abs()
                < 2
        );
    }
    #[test]
    fn shutdown_gates_late_plans_and_includes_retired_refresh_owners() {
        let (_dir, mut service, account, _) = fixture();
        let plan = service.sync_plan(&account.id).unwrap();
        let ProtocolCredential::ManagedOAuth2(old) = plan.login.credential else {
            panic!()
        };
        service
            .reauthorize_oauth(&account.id, 1, verified("stable-subject"), registration())
            .unwrap();
        let owners = service.oauth_managers_for_shutdown();
        assert!(owners.iter().any(|manager| Arc::ptr_eq(manager, &old)));
        assert!(service.sync_plan(&account.id).is_err());
    }
    #[test]
    fn recovered_connection_journal_invalidates_an_existing_oauth_manager() {
        let (_dir, mut service, account, _) = fixture();
        let plan = service.sync_plan(&account.id).unwrap();
        let ProtocolCredential::ManagedOAuth2(manager) = plan.login.credential else {
            panic!()
        };
        service.storage.db.execute_batch("CREATE TRIGGER fail_oauth_apply BEFORE UPDATE ON accounts BEGIN SELECT RAISE(FAIL, 'fixture write failure'); END;").unwrap();
        assert!(
            service
                .reauthorize_oauth(&account.id, 1, verified("stable-subject"), registration())
                .is_err()
        );
        assert!(manager.is_valid());
        assert_eq!(service.storage.journals().unwrap().len(), 1);
        service
            .storage
            .db
            .execute_batch("DROP TRIGGER fail_oauth_apply;")
            .unwrap();
        service.recover().unwrap();
        assert!(!manager.is_valid());
        assert_eq!(service.get(&account.id).unwrap().revision, 2);
    }
    #[test]
    fn normalization_precedes_mail_policy_and_arbitrary_mail_targets_are_rejected() {
        let mut request = request();
        request.config.email_address = "  lin@example.com  ".into();
        request.config.imap.username.clear();
        request.config.smtp.username = "  ".into();
        normalize_oauth_request(&mut request).unwrap();
        assert_eq!(request.config.email_address, "lin@example.com");
        assert_eq!(request.config.imap.username, "lin@example.com");
        let mut reg = registration();
        reg.mail_policy.imap.login = crate::auth::MailLoginPolicy::Mailbox;
        reg.mail_policy.smtp.login = crate::auth::MailLoginPolicy::Mailbox;
        assert!(
            reg.validate_mail_target(
                &request.config.email_address,
                &request.config.imap,
                &request.config.smtp
            )
            .is_ok()
        );
        for changed in [0, 1, 2, 3] {
            let mut config = request.config.imap.clone();
            match changed {
                0 => config.host = "attacker.invalid".into(),
                1 => config.port = 443,
                2 => config.tls = TlsMode::StartTls,
                _ => config.username = "attacker@example.com".into(),
            };
            assert!(
                reg.validate_mail_target(
                    &request.config.email_address,
                    &config,
                    &request.config.smtp
                )
                .is_err()
            );
        }
    }
    #[test]
    fn explicit_retry_restarts_budget_and_reports_authorization_resume() {
        let (_dir, mut service, account, _) = fixture();
        let now = chrono::Utc::now();
        let item = queued(&mut service, &account, now);
        let super::super::Next::Job { .. } = service.outbox_step(&account.id, now).unwrap().next
        else {
            panic!()
        };
        service
            .outbox_requeue(
                &item.id,
                now,
                &crate::send::types::OutboxError::reason("connect"),
                now,
            )
            .unwrap();
        let later = now + chrono::Duration::days(3);
        service
            .record_oauth_send_blocked(&account.id, "authorizationRequired", now)
            .unwrap();
        service.storage.retry_now(&item.id, later).unwrap();
        let changed = service
            .clear_oauth_send_blocked(&account.id, later)
            .unwrap();
        assert_eq!(changed.len(), 1);
        assert!(changed[0].item.error.is_none());
        let super::super::Next::Job { job, .. } =
            service.outbox_step(&account.id, later).unwrap().next
        else {
            panic!()
        };
        assert!((job.retry_started_at - later).num_milliseconds().abs() < 2);
    }

    #[tokio::test]
    async fn removal_retains_durable_cleanup_until_a_racing_rotation_finishes() {
        use crate::secrets::{SecretStore, SecretStoreError};
        use std::sync::{
            Condvar, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        struct RacingStore {
            inner: MemorySecrets,
            block: AtomicBool,
            delete_fails: AtomicBool,
            entered: tokio::sync::Notify,
            released: Mutex<bool>,
            release: Condvar,
        }
        impl SecretStore for RacingStore {
            fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
                self.inner.get(key)
            }
            fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
                if self.block.load(Ordering::SeqCst) {
                    self.entered.notify_one();
                    let mut released = self.released.lock().unwrap();
                    while !*released {
                        released = self.release.wait(released).unwrap();
                    }
                }
                self.inner.set(key, value)
            }
            fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
                if self.delete_fails.load(Ordering::SeqCst) {
                    Err(SecretStoreError::Unavailable)
                } else {
                    self.inner.delete(key)
                }
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 8192];
            let count = stream.read(&mut request).await.unwrap();
            assert!(count > 0);
            let body = r#"{"access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":3600,"token_type":"Bearer","scope":"mail"}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let dir = TempDir::new().unwrap();
        let secrets = Arc::new(RacingStore {
            inner: MemorySecrets::default(),
            block: AtomicBool::new(false),
            delete_fails: AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
            released: Mutex::new(false),
            release: Condvar::new(),
        });
        let mut service = AccountService::open(dir.path(), secrets.clone()).unwrap();
        let mut reg = registration();
        reg.token_endpoint = format!("http://127.0.0.1:{port}/token").parse().unwrap();
        let mut authorization = verified("stable-subject");
        authorization.tokens.expires_at = 0;
        let account = service.create_oauth(request(), authorization, reg).unwrap();
        let key = service.storage.load(&account.id).unwrap().profiles[0]
            .secret_ref
            .clone();
        let plan = service.sync_plan(&account.id).unwrap();
        let ProtocolCredential::ManagedOAuth2(manager) = plan.login.credential else {
            panic!()
        };
        secrets.block.store(true, Ordering::SeqCst);
        let entered = secrets.entered.notified();
        let refreshing = manager.clone();
        let refresh = tokio::spawn(async move {
            refreshing
                .access_token(&crate::net::NetRoute::default())
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), entered)
            .await
            .unwrap();
        let removal = service
            .remove(RemoveAccountRequest {
                account_id: account.id.clone(),
                expected_revision: 1,
            })
            .unwrap();
        assert!(removal.credentials_pending);
        assert!(service.storage.removed_secrets().unwrap().contains(&key));
        secrets.delete_fails.store(true, Ordering::SeqCst);
        *secrets.released.lock().unwrap() = true;
        secrets.release.notify_all();
        assert!(matches!(
            refresh.await.unwrap(),
            Err(crate::auth::OAuthError::Storage)
        ));
        manager.drain().await;
        assert!(
            secrets.get(&key).unwrap().is_some(),
            "failed compensation leaves key for durable retry"
        );
        assert!(service.storage.removed_secrets().unwrap().contains(&key));
        drop(service);
        secrets.block.store(false, Ordering::SeqCst);
        secrets.delete_fails.store(false, Ordering::SeqCst);
        let reopened = AccountService::open(dir.path(), secrets.clone()).unwrap();
        assert!(secrets.get(&key).unwrap().is_none());
        assert!(reopened.storage.removed_secrets().unwrap().is_empty());
        server.await.unwrap();
    }
    #[test]
    fn trusted_policy_rejects_coordinated_registration_and_binding_tampering() {
        for field in ["client_id", "registration_id"] {
            let (dir, service, account, secrets) = fixture();
            let snapshot = service.storage.load(&account.id).unwrap();
            let mut binding = snapshot.profiles[0].oauth.clone().unwrap();
            let mut registration = registration();
            if field == "client_id" {
                registration.client_id = "attacker-client".into();
                binding.client_id = registration.client_id.clone();
            } else {
                registration.registration_id = "attacker-registration".into();
                binding.client_registration_id = registration.registration_id.clone();
            }
            service
                .storage
                .db
                .execute(
                    "UPDATE oauth_client_registrations SET id=?1,configuration_json=?2",
                    rusqlite::params![
                        registration.registration_id,
                        crate::storage::encode(&registration).unwrap()
                    ],
                )
                .unwrap();
            service
                .storage
                .db
                .execute(
                    "UPDATE auth_profiles SET oauth_binding_json=?1 WHERE account_id=?2",
                    rusqlite::params![crate::storage::encode(&binding).unwrap(), account.id],
                )
                .unwrap();
            drop(service);
            let mut reopened = AccountService::open(dir.path(), secrets).unwrap();
            reopened.trust_test_oauth_policy(&self::registration());
            assert!(matches!(
                reopened.sync_plan(&account.id),
                Err(super::super::sync_plan::PlanError::CredentialUntrusted)
            ));
        }
    }
    #[test]
    fn mail_policy_rejects_coordinated_database_endpoint_and_trust_changes_with_cached_or_restarted_managers()
     {
        for protocol in ["imap", "smtp"] {
            for field in ["host", "port", "tls", "username"] {
                let (dir, service, account, secrets) = fixture();
                service.sync_plan(&account.id).unwrap();
                let mut snapshot = service.storage.load(&account.id).unwrap();
                let connection = if protocol == "imap" {
                    &mut snapshot.account.imap
                } else {
                    &mut snapshot.account.smtp
                };
                match field {
                    "host" => connection.host = "attacker.invalid".into(),
                    "port" => connection.port = 443,
                    "tls" => connection.tls = TlsMode::StartTls,
                    _ => connection.username = "attacker@example.com".into(),
                }
                service.storage.db.execute("UPDATE mail_connections SET host=?3,port=?4,tls=?5,username=?6 WHERE account_id=?1 AND protocol=?2",rusqlite::params![account.id,protocol,connection.host,connection.port,if connection.tls==TlsMode::Implicit{"implicit"}else{"startTls"},connection.username]).unwrap();
                snapshot.profiles[0].trusted_endpoints = vec![
                    TrustEndpoint::mail("imap", &snapshot.account.imap),
                    TrustEndpoint::mail("smtp", &snapshot.account.smtp),
                ];
                service
                    .storage
                    .db
                    .execute(
                        "UPDATE auth_profiles SET trusted_endpoints_json=?1 WHERE account_id=?2",
                        rusqlite::params![
                            crate::storage::encode(&snapshot.profiles[0].trusted_endpoints)
                                .unwrap(),
                            account.id
                        ],
                    )
                    .unwrap();
                assert!(
                    matches!(
                        service.sync_plan(&account.id),
                        Err(super::super::sync_plan::PlanError::CredentialUntrusted)
                    ),
                    "cached {protocol}.{field}"
                );
                assert!(
                    matches!(
                        service.smtp_plan(&account.id),
                        Err(super::super::sync_plan::PlanError::CredentialUntrusted)
                    ),
                    "cached SMTP {protocol}.{field}"
                );
                drop(service);
                let mut reopened = AccountService::open(dir.path(), secrets).unwrap();
                reopened.trust_test_oauth_policy(&registration());
                assert!(
                    matches!(
                        reopened.sync_plan(&account.id),
                        Err(super::super::sync_plan::PlanError::CredentialUntrusted)
                    ),
                    "restart {protocol}.{field}"
                );
                assert!(
                    matches!(
                        reopened.smtp_plan(&account.id),
                        Err(super::super::sync_plan::PlanError::CredentialUntrusted)
                    ),
                    "restart SMTP {protocol}.{field}"
                );
            }
        }
    }
}
