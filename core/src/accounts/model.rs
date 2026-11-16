use super::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TrustEndpoint {
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub tls: Option<TlsMode>,
    pub username: String,
    /// Only HTTPS mail protocols use a path; absent in historical journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}
impl TrustEndpoint {
    pub fn mail(protocol: &str, connection: &ConnectionConfig) -> Self {
        Self {
            protocol: protocol.into(),
            host: connection.host.clone(),
            port: connection.port,
            tls: Some(connection.tls),
            username: connection.username.clone(),
            path: None,
        }
    }

    pub fn ews(config: &EwsConnectionConfig) -> Result<Self, crate::AppError> {
        let url =
            url::Url::parse(&config.url).map_err(|_| crate::AppError::validation("ews.url"))?;
        Ok(Self {
            protocol: "ewsNtlm".into(),
            host: url.host_str().unwrap_or_default().to_owned(),
            port: url.port_or_known_default().unwrap_or(443),
            tls: Some(TlsMode::Implicit),
            username: config.username.clone(),
            path: Some(url.path().to_owned()),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AuthRecord {
    pub id: String,
    pub generation: u32,
    pub secret_ref: String,
    pub trusted_endpoints: Vec<TrustEndpoint>,
    #[serde(default)]
    pub oauth: Option<crate::auth::GrantBinding>,
}
impl AuthRecord {
    pub fn summary(&self, endpoint: &TrustEndpoint) -> AuthenticationSummary {
        AuthenticationSummary {
            id: self.id.clone(),
            generation: self.generation,
            password_configured: true,
            trust: self.trust(endpoint),
        }
    }
    pub fn trust(&self, endpoint: &TrustEndpoint) -> CredentialTrust {
        if self.trusted_endpoints.contains(endpoint) {
            CredentialTrust::Trusted
        } else {
            CredentialTrust::NeedsConfirmation
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProxySecret {
    pub secret_ref: String,
    pub generation: u32,
    pub trusted_endpoint: TrustEndpoint,
}

/// Non-sensitive durable intent, sufficient to finish a local commit after a crash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub account: Account,
    pub profiles: Vec<AuthRecord>,
    pub proxy_secret: Option<ProxySecret>,
}
impl Snapshot {
    pub fn refs(&self) -> Vec<String> {
        self.profiles
            .iter()
            .map(|p| p.secret_ref.clone())
            .chain(self.proxy_secret.iter().map(|p| p.secret_ref.clone()))
            .collect()
    }
    pub fn profile(&self, id: &str) -> Option<&AuthRecord> {
        self.profiles.iter().find(|profile| profile.id == id)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Journal {
    pub id: String,
    pub expected_revision: Option<u32>,
    pub snapshot: Snapshot,
    pub new_refs: Vec<String>,
    pub retired_refs: Vec<String>,
    pub create_request: Option<(String, String)>,
    pub applied: bool,
}
