pub(crate) mod bodies;
#[cfg(any(test, feature = "body-webview-test"))]
mod body_fixture;
pub(crate) mod messages;
pub(crate) mod model;
pub(crate) mod sync_plan;
pub mod types;

use crate::{
    AppError, ErrorCode,
    secrets::{SecretStore, SecretValue},
    storage::{Storage, encode, proxy_endpoint},
};
use model::*;
use std::{path::Path, sync::Arc};
use types::*;
use uuid::Uuid;

/// Synchronous application service. Hosts must run it off their UI/event thread.
/// Opening owns the data-directory lock until this service is dropped.
pub struct AccountService {
    storage: Storage,
    secrets: Arc<dyn SecretStore>,
}

impl AccountService {
    pub fn open(path: &Path, secrets: Arc<dyn SecretStore>) -> Result<Self, AppError> {
        let mut service = Self {
            storage: Storage::open(path)?,
            secrets,
        };
        service.recover()?;
        Ok(service)
    }

    pub fn list(&self) -> Result<Vec<Account>, AppError> {
        self.storage
            .account_ids()?
            .iter()
            .map(|id| self.get(id))
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<Account, AppError> {
        validate_uuid(id, "accountId")?;
        let snapshot = self.storage.load(id)?;
        // Read only presence from the backend; never return its value or reference.
        let mut account = snapshot.account;
        for summary in [
            &mut account.authentication.imap,
            &mut account.authentication.smtp,
        ] {
            let profile = snapshot
                .profiles
                .iter()
                .find(|p| p.id == summary.id)
                .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
            summary.password_configured = self.secrets.get(&profile.secret_ref)?.is_some();
        }
        if let RouteConfig::Custom {
            authentication: Some(auth),
            ..
        } = &mut account.route
        {
            auth.password_configured = if let Some(secret) = snapshot.proxy_secret {
                self.secrets.get(&secret.secret_ref)?.is_some()
            } else {
                false
            };
        }
        Ok(account)
    }

    pub fn create(&mut self, mut request: CreateAccountRequest) -> Result<Account, AppError> {
        validate_uuid(&request.request_id, "requestId")?;
        validate_config(&mut request.config)?;
        validate_create_secrets(&request.config)?;
        self.recover()?;
        let canonical = canonical_config(&request.config)?;
        if let Some((id, saved)) = self.storage.create_request(&request.request_id)? {
            let snapshot = self.storage.load(&id)?;
            // A historical request cannot be replayed against later account edits.
            if saved != canonical
                || snapshot.account.revision != 1
                || !self.matches_secrets(&snapshot, &request.config)?
            {
                return Err(AppError::new(ErrorCode::IdempotencyConflict));
            }
            return self.get(&id);
        }
        let (snapshot, writes) = self.build(request.config, None)?;
        let journal = Journal {
            id: new_id(),
            expected_revision: None,
            new_refs: writes.iter().map(|w| w.0.clone()).collect(),
            retired_refs: vec![],
            snapshot,
            create_request: Some((request.request_id, canonical)),
            applied: false,
        };
        self.commit(journal, writes)
    }

    pub fn update(&mut self, mut request: UpdateAccountRequest) -> Result<Account, AppError> {
        validate_uuid(&request.account_id, "accountId")?;
        validate_config(&mut request.config)?;
        self.recover()?;
        let old = self.storage.load(&request.account_id)?;
        if old.account.revision != request.expected_revision {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        let (snapshot, writes) = self.build(request.config, Some(&old))?;
        let current_refs = snapshot.refs();
        let journal = Journal {
            id: new_id(),
            expected_revision: Some(request.expected_revision),
            new_refs: writes.iter().map(|w| w.0.clone()).collect(),
            retired_refs: old
                .refs()
                .into_iter()
                .filter(|key| !current_refs.contains(key))
                .collect(),
            snapshot,
            create_request: None,
            applied: false,
        };
        self.commit(journal, writes)
    }

    fn commit(
        &mut self,
        mut journal: Journal,
        writes: Vec<(String, SecretValue)>,
    ) -> Result<Account, AppError> {
        // The intent is durable before a new credential can become orphaned.
        self.storage.prepare(&journal)?;
        for (key, value) in writes {
            self.secrets.set(&key, &value)?;
        }
        // Confirm every referenced credential exists before confirming the config.
        for key in journal.snapshot.refs() {
            if self.secrets.get(&key)?.is_none() {
                return Err(AppError::new(ErrorCode::SecretMissing));
            }
        }
        self.storage.apply(&journal)?;
        journal.applied = true;
        // Cleanup failures are safe to defer: the committed new configuration is
        // already durable and a retained journal will retry old, unreferenced keys.
        self.cleanup(&journal).ok();
        Ok(journal.snapshot.account)
    }

    fn recover(&mut self) -> Result<(), AppError> {
        for mut journal in self.storage.journals()? {
            if journal.applied {
                self.cleanup(&journal).ok();
                continue;
            }
            let current_revision = match self.storage.load(&journal.snapshot.account.id) {
                Ok(snapshot) => Some(snapshot.account.revision),
                Err(error) if error.code == ErrorCode::NotFound => None,
                Err(error) => return Err(error),
            };
            // Collect availability before deciding anything: errors must not be
            // mistaken for absent secrets or lead to destructive cleanup.
            let mut complete = true;
            for key in journal.snapshot.refs() {
                if self.secrets.get(&key)?.is_none() {
                    complete = false;
                }
            }
            if complete && current_revision == journal.expected_revision {
                self.storage.apply(&journal)?;
                journal.applied = true;
                self.cleanup(&journal).ok();
            } else {
                for key in &journal.new_refs {
                    if !self.storage.is_referenced(key)? {
                        self.secrets.delete(key)?;
                    }
                }
                self.storage.remove_journal(&journal.id)?;
            }
        }
        Ok(())
    }

    fn cleanup(&self, journal: &Journal) -> Result<(), AppError> {
        for key in &journal.retired_refs {
            if !self.storage.is_referenced(key)? {
                self.secrets.delete(key)?;
            }
        }
        self.storage.remove_journal(&journal.id)
    }

    fn build(
        &self,
        config: AccountConfigInput,
        old: Option<&Snapshot>,
    ) -> Result<(Snapshot, Vec<(String, SecretValue)>), AppError> {
        let id = old.map(|s| s.account.id.clone()).unwrap_or_else(new_id);
        let imap_endpoint = TrustEndpoint::mail("imap", &config.imap);
        let smtp_endpoint = TrustEndpoint::mail("smtp", &config.smtp);
        let old_imap = old.and_then(|s| s.profile(&s.account.authentication.imap.id));
        let old_smtp = old.and_then(|s| s.profile(&s.account.authentication.smtp.id));
        let mut writes = vec![];
        let AuthenticationInput::Password { credentials } = config.authentication else {
            return Err(AppError::new(ErrorCode::Unsupported));
        };
        let (profiles, mode, imap_summary, smtp_summary) = match credentials {
            PasswordAuthenticationInput::Shared { password } => {
                let profile = self.build_profile(
                    &id,
                    old_imap,
                    password,
                    &[imap_endpoint.clone(), smtp_endpoint.clone()],
                    false,
                    &mut writes,
                )?;
                let imap = profile.summary(&imap_endpoint);
                let smtp = profile.summary(&smtp_endpoint);
                (vec![profile], AuthenticationMode::Shared, imap, smtp)
            }
            PasswordAuthenticationInput::Separate {
                imap_password,
                smtp_password,
            } => {
                let imap = self.build_profile(
                    &id,
                    old_imap,
                    imap_password,
                    std::slice::from_ref(&imap_endpoint),
                    false,
                    &mut writes,
                )?;
                let copy = old_imap.zip(old_smtp).is_some_and(|(a, b)| a.id == b.id);
                let smtp = self.build_profile(
                    &id,
                    old_smtp,
                    smtp_password,
                    std::slice::from_ref(&smtp_endpoint),
                    copy,
                    &mut writes,
                )?;
                let imap_summary = imap.summary(&imap_endpoint);
                let smtp_summary = smtp.summary(&smtp_endpoint);
                (
                    vec![imap, smtp],
                    AuthenticationMode::Separate,
                    imap_summary,
                    smtp_summary,
                )
            }
        };
        let (route, proxy_secret) = self.build_route(&id, config.route, old, &mut writes)?;
        let route_changed = old.is_some_and(|s| {
            s.account.route != route
                || s.proxy_secret.as_ref().map(|v| &v.secret_ref)
                    != proxy_secret.as_ref().map(|v| &v.secret_ref)
        });
        let route_revision = match old {
            Some(s) if route_changed => next_version(s.account.route_revision)?,
            Some(s) => s.account.route_revision,
            None => 1,
        };
        let source = old
            .filter(|s| same_source(&s.account.imap, &config.imap))
            .map(|s| s.account.mailbox_source_id.clone())
            .unwrap_or_else(new_id);
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let account = Account {
            id,
            revision: old
                .map(|s| next_version(s.account.revision))
                .transpose()?
                .unwrap_or(1),
            display_name: config.display_name,
            email_address: config.email_address,
            provider_id: config.provider_id,
            mailbox_source_id: source,
            imap: config.imap,
            smtp: config.smtp,
            authentication: PasswordAuthenticationSummary {
                mode,
                imap: imap_summary,
                smtp: smtp_summary,
            },
            default_route_revision: if matches!(route, RouteConfig::Inherit) {
                Some(1)
            } else {
                None
            },
            route,
            route_revision,
            connection_status: ConnectionStatus::Unverified,
            created_at: old
                .map(|s| s.account.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        Ok((
            Snapshot {
                account,
                profiles,
                proxy_secret,
            },
            writes,
        ))
    }

    fn build_profile(
        &self,
        account_id: &str,
        old: Option<&AuthRecord>,
        update: SecretUpdate,
        endpoints: &[TrustEndpoint],
        copy: bool,
        writes: &mut Vec<(String, SecretValue)>,
    ) -> Result<AuthRecord, AppError> {
        match update {
            SecretUpdate::Keep => {
                let old = old.ok_or_else(|| AppError::validation("authentication"))?;
                let value = self
                    .secrets
                    .get(&old.secret_ref)?
                    .ok_or_else(|| AppError::new(ErrorCode::SecretMissing))?;
                if !copy {
                    return Ok(old.clone());
                }
                let mut profile = old.clone();
                profile.id = new_id();
                profile.secret_ref = secret_key(account_id);
                profile.generation = 1;
                writes.push((profile.secret_ref.clone(), value));
                Ok(profile)
            }
            SecretUpdate::Replace { value } => {
                validate_secret(&value, "authentication")?;
                let profile = AuthRecord {
                    id: old
                        .filter(|_| !copy)
                        .map(|p| p.id.clone())
                        .unwrap_or_else(new_id),
                    generation: old
                        .filter(|_| !copy)
                        .map(|p| next_version(p.generation))
                        .transpose()?
                        .unwrap_or(1),
                    secret_ref: secret_key(account_id),
                    trusted_endpoints: endpoints.to_vec(),
                };
                writes.push((profile.secret_ref.clone(), value));
                Ok(profile)
            }
        }
    }

    fn build_route(
        &self,
        account_id: &str,
        input: RouteInput,
        old: Option<&Snapshot>,
        writes: &mut Vec<(String, SecretValue)>,
    ) -> Result<(RouteConfig, Option<ProxySecret>), AppError> {
        match input {
            RouteInput::Direct => Ok((RouteConfig::Direct, None)),
            RouteInput::Inherit => Ok((RouteConfig::Inherit, None)),
            RouteInput::Custom {
                proxy_kind,
                host,
                port,
                dns,
                authentication,
            } => {
                let old_secret = old.and_then(|s| s.proxy_secret.as_ref());
                let mut proxy_secret = None;
                let authentication = if let Some(auth) = authentication {
                    let endpoint = proxy_endpoint(proxy_kind, &host, port, &auth.username);
                    let secret = match auth.password {
                        SecretUpdate::Keep => {
                            let secret = old_secret
                                .ok_or_else(|| AppError::validation("route.authentication"))?;
                            if self.secrets.get(&secret.secret_ref)?.is_none() {
                                return Err(AppError::new(ErrorCode::SecretMissing));
                            }
                            secret.clone()
                        }
                        SecretUpdate::Replace { value } => {
                            validate_secret(&value, "route.authentication.password")?;
                            let secret = ProxySecret {
                                secret_ref: secret_key(account_id),
                                generation: old_secret
                                    .map(|s| next_version(s.generation))
                                    .transpose()?
                                    .unwrap_or(1),
                                trusted_endpoint: endpoint.clone(),
                            };
                            writes.push((secret.secret_ref.clone(), value));
                            secret
                        }
                    };
                    let trust = if secret.trusted_endpoint == endpoint {
                        CredentialTrust::Trusted
                    } else {
                        CredentialTrust::NeedsConfirmation
                    };
                    proxy_secret = Some(secret);
                    Some(ProxyAuthenticationSummary {
                        username: auth.username,
                        password_configured: true,
                        trust,
                    })
                } else {
                    None
                };
                Ok((
                    RouteConfig::Custom {
                        proxy_kind,
                        host,
                        port,
                        dns,
                        authentication,
                    },
                    proxy_secret,
                ))
            }
        }
    }

    fn matches_secrets(
        &self,
        snapshot: &Snapshot,
        config: &AccountConfigInput,
    ) -> Result<bool, AppError> {
        let mut comparisons = vec![];
        let AuthenticationInput::Password { credentials } = &config.authentication else {
            return Ok(false);
        };
        let imap = snapshot
            .profile(&snapshot.account.authentication.imap.id)
            .ok_or_else(|| AppError::new(ErrorCode::Internal))?;
        let smtp = snapshot
            .profile(&snapshot.account.authentication.smtp.id)
            .ok_or_else(|| AppError::new(ErrorCode::Internal))?;
        match credentials {
            PasswordAuthenticationInput::Shared { password } => {
                comparisons.push((&imap.secret_ref, password))
            }
            PasswordAuthenticationInput::Separate {
                imap_password,
                smtp_password,
            } => {
                comparisons.push((&imap.secret_ref, imap_password));
                comparisons.push((&smtp.secret_ref, smtp_password));
            }
        }
        if let RouteInput::Custom {
            authentication: Some(auth),
            ..
        } = &config.route
        {
            let secret = snapshot
                .proxy_secret
                .as_ref()
                .ok_or_else(|| AppError::new(ErrorCode::Internal))?;
            comparisons.push((&secret.secret_ref, &auth.password));
        }
        for (key, update) in comparisons {
            let SecretUpdate::Replace { value } = update else {
                return Ok(false);
            };
            let saved = self
                .secrets
                .get(key)?
                .ok_or_else(|| AppError::new(ErrorCode::SecretMissing))?;
            if saved.expose_secret() != value.expose_secret() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}
fn next_version(current: u32) -> Result<u32, AppError> {
    current
        .checked_add(1)
        .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))
}
fn secret_key(account_id: &str) -> String {
    format!("mailx/{account_id}/{}", new_id())
}
fn same_source(a: &ConnectionConfig, b: &ConnectionConfig) -> bool {
    a.host == b.host && a.port == b.port && a.username == b.username
}
fn validate_uuid(value: &str, field: &str) -> Result<(), AppError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| AppError::validation(field))
}
fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
fn validate_secret(value: &SecretValue, field: &str) -> Result<(), AppError> {
    let text = value.expose_secret();
    if text.trim().is_empty()
        || text.len() > 16384
        || text.chars().all(|c| c == '*' || c == '•')
        || text.contains('\0')
    {
        return Err(AppError::validation(field));
    }
    Ok(())
}
fn validate_host(host: &mut String, field: &str) -> Result<(), AppError> {
    *host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    if host.is_empty()
        || host.len() > 253
        || !host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(AppError::validation(field));
    }
    Ok(())
}
fn validate_config(config: &mut AccountConfigInput) -> Result<(), AppError> {
    if !matches!(config.authentication, AuthenticationInput::Password { .. }) {
        return Err(AppError::new(ErrorCode::Unsupported));
    }
    config.display_name = config.display_name.trim().to_owned();
    config.email_address = config.email_address.trim().to_owned();
    if !valid_text(&config.display_name, 120) {
        return Err(AppError::validation("displayName"));
    }
    if !valid_text(&config.email_address, 320)
        || config.email_address.chars().any(char::is_whitespace)
    {
        return Err(AppError::validation("emailAddress"));
    }
    let parts: Vec<_> = config.email_address.split('@').collect();
    if parts.len() != 2 || parts[0].is_empty() {
        return Err(AppError::validation("emailAddress"));
    }
    let mut domain = parts[1].to_owned();
    validate_host(&mut domain, "emailAddress")?;
    if config.provider_id.as_ref().is_some_and(|id| {
        id.is_empty()
            || id.len() > 80
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    }) {
        return Err(AppError::validation("providerId"));
    }
    for (name, connection) in [("imap", &mut config.imap), ("smtp", &mut config.smtp)] {
        validate_host(&mut connection.host, &format!("{name}.host"))?;
        if connection.port == 0 {
            return Err(AppError::validation(&format!("{name}.port")));
        }
        if !valid_text(&connection.username, 320) {
            return Err(AppError::validation(&format!("{name}.username")));
        }
    }
    if let RouteInput::Custom {
        host,
        port,
        authentication,
        ..
    } = &mut config.route
    {
        validate_host(host, "route.host")?;
        if *port == 0 {
            return Err(AppError::validation("route.port"));
        }
        if let Some(auth) = authentication {
            if !valid_text(&auth.username, 255) {
                return Err(AppError::validation("route.authentication.username"));
            }
            if let SecretUpdate::Replace { value } = &auth.password {
                validate_secret(value, "route.authentication.password")?;
            }
        }
    }
    let AuthenticationInput::Password { credentials } = &config.authentication else {
        unreachable!()
    };
    for update in match credentials {
        PasswordAuthenticationInput::Shared { password } => vec![password],
        PasswordAuthenticationInput::Separate {
            imap_password,
            smtp_password,
        } => vec![imap_password, smtp_password],
    } {
        if let SecretUpdate::Replace { value } = update {
            validate_secret(value, "authentication")?;
        }
    }
    Ok(())
}

fn validate_create_secrets(config: &AccountConfigInput) -> Result<(), AppError> {
    let AuthenticationInput::Password { credentials } = &config.authentication else {
        return Err(AppError::new(ErrorCode::Unsupported));
    };
    let any_keep = match credentials {
        PasswordAuthenticationInput::Shared { password } => matches!(password, SecretUpdate::Keep),
        PasswordAuthenticationInput::Separate {
            imap_password,
            smtp_password,
        } => {
            matches!(imap_password, SecretUpdate::Keep)
                || matches!(smtp_password, SecretUpdate::Keep)
        }
    };
    if any_keep {
        return Err(AppError::validation("authentication"));
    }
    if matches!(
        &config.route,
        RouteInput::Custom {
            authentication: Some(ProxyAuthenticationInput {
                password: SecretUpdate::Keep,
                ..
            }),
            ..
        }
    ) {
        return Err(AppError::validation("route.authentication"));
    }
    Ok(())
}

/// No secret, password digest, or secret-derived value is included in this record.
fn canonical_config(config: &AccountConfigInput) -> Result<String, AppError> {
    let mode = match &config.authentication {
        AuthenticationInput::Password {
            credentials: PasswordAuthenticationInput::Shared { .. },
        } => "shared",
        AuthenticationInput::Password {
            credentials: PasswordAuthenticationInput::Separate { .. },
        } => "separate",
        _ => return Err(AppError::new(ErrorCode::Unsupported)),
    };
    let route = match &config.route {
        RouteInput::Direct => serde_json::json!({"kind":"direct"}),
        RouteInput::Inherit => serde_json::json!({"kind":"inherit"}),
        RouteInput::Custom {
            proxy_kind,
            host,
            port,
            dns,
            authentication,
        } => {
            serde_json::json!({"kind":"custom","proxyKind":proxy_kind,"host":host,"port":port,"dns":dns,"username":authentication.as_ref().map(|a|&a.username)})
        }
    };
    encode(
        &serde_json::json!({"displayName":config.display_name,"emailAddress":config.email_address,"providerId":config.provider_id,"imap":config.imap,"smtp":config.smtp,"authenticationMode":mode,"route":route}),
    )
}
