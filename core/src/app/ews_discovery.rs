//! Read-only discovery, with single-use grants for exact credential recipients.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::MailxCore;
use crate::accounts::types::{EwsConnectionConfig, RouteInput};
use crate::accounts::{draft_connection::route_from_input, normalize_email_address};
use crate::error::{AppError, ErrorCode};
use crate::ews::{EwsError, discovery};
use crate::net::NetError;
use crate::secrets::SecretValue;

const GRANT_TTL: Duration = Duration::from_secs(300);
const MAX_GRANTS: usize = 16;

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoverEwsSettings {
    pub email_address: String,
    pub route: RouteInput,
    #[serde(default)]
    pub target_grant: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub password: Option<SecretValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EwsDiscoveryStatus {
    Found,
    AuthRequired,
    ConfirmationRequired,
    NotFound,
    Rejected,
    Network,
    Proxy,
    Tls,
    Timeout,
    Unsupported,
    Protocol,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EwsSettingsDiscovery {
    pub status: EwsDiscoveryStatus,
    pub endpoint: Option<EwsConnectionConfig>,
    pub source: Option<String>,
    pub target_grant: Option<String>,
}

struct Grant {
    url: String,
    email: String,
    expires: Instant,
}
#[derive(Default)]
pub(super) struct DiscoveryRegistry(Mutex<HashMap<String, Grant>>);
impl DiscoveryRegistry {
    fn issue(&self, url: String, email: String) -> Result<String, AppError> {
        // Only verified HTTPS URLs can become credential recipients.
        crate::ews::Endpoint::parse(&url).map_err(|_| AppError::new(ErrorCode::Unsupported))?;
        let mut grants = self
            .0
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        grants.retain(|_, grant| grant.expires > Instant::now());
        if grants.len() >= MAX_GRANTS {
            grants.clear();
        }
        let id = Uuid::new_v4().to_string();
        grants.insert(
            id.clone(),
            Grant {
                url,
                email,
                expires: Instant::now() + GRANT_TTL,
            },
        );
        Ok(id)
    }
    fn consume(&self, id: &str, email: &str) -> Result<String, AppError> {
        let mut grants = self
            .0
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        let grant = grants
            .remove(id)
            .ok_or_else(|| AppError::validation("targetGrant"))?;
        if grant.email != email || grant.expires <= Instant::now() {
            return Err(AppError::validation("targetGrant"));
        }
        Ok(grant.url)
    }
}

impl MailxCore {
    pub async fn discover_ews_settings(
        &self,
        request: DiscoverEwsSettings,
    ) -> Result<EwsSettingsDiscovery, AppError> {
        let email = normalize_email_address(&request.email_address)?;
        let route = route_from_input(request.route)?;
        // Callers cannot turn a broad discovery lookup into a password spray.
        if request.password.is_some() && request.target_grant.is_none() {
            return Err(AppError::validation("targetGrant"));
        }
        let target = request
            .target_grant
            .as_deref()
            .map(|grant| self.ews_discovery.consume(grant, &email))
            .transpose()?;
        let username = request.username.as_deref().unwrap_or(&email).trim();
        if username.is_empty() || username.len() > 320 || username.chars().any(char::is_control) {
            return Err(AppError::validation("ews.username"));
        }
        if request.password.as_ref().is_some_and(|value| {
            value.expose_secret().is_empty() || value.expose_secret().len() > 16_384
        }) {
            return Err(AppError::validation("sharedPassword"));
        }
        let result = discovery::discover(
            &route,
            &email,
            target.as_deref(),
            request.password.as_ref().map(|secret| (username, secret)),
        )
        .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                return Ok(EwsSettingsDiscovery {
                    status: discovery_failure(error),
                    endpoint: None,
                    source: target,
                    target_grant: None,
                });
            }
        };
        let (status, recipient) = if let Some(redirect) = &result.redirect {
            (
                EwsDiscoveryStatus::ConfirmationRequired,
                Some(redirect.clone()),
            )
        } else if result.authentication_required {
            if matches!(
                result.auth,
                discovery::DiscoveryAuth::Ntlm
                    | discovery::DiscoveryAuth::Negotiate
                    | discovery::DiscoveryAuth::Basic
            ) {
                (
                    EwsDiscoveryStatus::AuthRequired,
                    Some(result.source.clone()),
                )
            } else {
                (EwsDiscoveryStatus::Unsupported, None)
            }
        } else if result.endpoint.is_some() {
            (EwsDiscoveryStatus::Found, None)
        } else {
            (EwsDiscoveryStatus::NotFound, None)
        };
        let source = recipient.clone().or(Some(result.source));
        let target_grant = recipient
            .map(|url| self.ews_discovery.issue(url, email.clone()))
            .transpose()?;
        Ok(EwsSettingsDiscovery {
            status,
            endpoint: result
                .endpoint
                .map(|url| found_config(url, result.login, username)),
            source,
            target_grant,
        })
    }
}

/// Without a server-provided LoginName, keep the username the caller entered
/// (e.g. `DOMAIN\user`); it defaults to the email address.
fn found_config(url: String, login: Option<String>, username: &str) -> EwsConnectionConfig {
    EwsConnectionConfig {
        url,
        username: login.unwrap_or_else(|| username.to_owned()),
    }
}

fn discovery_failure(error: EwsError) -> EwsDiscoveryStatus {
    match error {
        EwsError::Net(NetError::Proxy(_)) => EwsDiscoveryStatus::Proxy,
        EwsError::Net(NetError::Tls(_)) => EwsDiscoveryStatus::Tls,
        EwsError::Net(NetError::Timeout) | EwsError::Timeout => EwsDiscoveryStatus::Timeout,
        EwsError::Net(_) => EwsDiscoveryStatus::Network,
        EwsError::Authentication => EwsDiscoveryStatus::Rejected,
        EwsError::AuthUnsupported => EwsDiscoveryStatus::Unsupported,
        EwsError::NotFound => EwsDiscoveryStatus::NotFound,
        _ => EwsDiscoveryStatus::Protocol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grants_are_bound_to_one_email_and_one_exact_target_and_consumed_once() {
        let registry = DiscoveryRegistry::default();
        let url = "https://autodiscover.example.com/autodiscover/autodiscover.xml";
        let id = registry
            .issue(url.into(), "alice@example.com".into())
            .unwrap();
        assert!(registry.consume(&id, "other@example.com").is_err());
        assert!(registry.consume(&id, "alice@example.com").is_err());
        let id = registry
            .issue(url.into(), "alice@example.com".into())
            .unwrap();
        assert_eq!(registry.consume(&id, "alice@example.com").unwrap(), url);
        assert!(registry.consume(&id, "alice@example.com").is_err());
        assert!(
            registry
                .issue("http://example.com/".into(), "alice@example.com".into())
                .is_err()
        );
    }
    #[test]
    fn an_expired_grant_is_never_used() {
        let registry = DiscoveryRegistry::default();
        registry.0.lock().unwrap().insert(
            "expired".into(),
            Grant {
                url: "https://example.com/".into(),
                email: "alice@example.com".into(),
                expires: Instant::now() - Duration::from_secs(1),
            },
        );
        assert!(registry.consume("expired", "alice@example.com").is_err());
    }
    #[test]
    fn found_settings_keep_the_entered_username_without_a_login_name() {
        let url = "https://mail.example.com/EWS/Exchange.asmx".to_owned();
        assert_eq!(
            found_config(url.clone(), None, "CORP\\alice").username,
            "CORP\\alice"
        );
        assert_eq!(
            found_config(url, Some("alice@corp.example".into()), "CORP\\alice").username,
            "alice@corp.example"
        );
    }
    #[tokio::test]
    async fn a_password_without_a_host_grant_is_rejected_before_network() {
        let directory = tempfile::TempDir::new().unwrap();
        let core = MailxCore::open(
            directory.path(),
            std::sync::Arc::new(crate::secrets::test_support::MemorySecrets::default()),
            tokio::runtime::Handle::current(),
            std::sync::Arc::new(|_: &crate::CoreEvent| {}),
        )
        .unwrap();
        let request = DiscoverEwsSettings {
            email_address: "alice@example.com".into(),
            route: RouteInput::Direct,
            target_grant: None,
            username: None,
            password: Some(SecretValue::new("synthetic-secret".into())),
        };
        let error = core.discover_ews_settings(request).await.unwrap_err();
        assert_eq!(error.field.as_deref(), Some("targetGrant"));
        // A raw caller-provided URL cannot stand in for a grant.
        assert!(serde_json::from_value::<DiscoverEwsSettings>(serde_json::json!({"emailAddress":"alice@example.com","route":{"kind":"direct"},"targetUrl":"https://example.com/","password":"synthetic-secret"})).is_err());
    }
}
