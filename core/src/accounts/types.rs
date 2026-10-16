//! Public wire contract. Input secrets cannot be serialized back to clients.
use crate::secrets::SecretValue;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TlsMode {
    Implicit,
    StartTls,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub username: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum SecretUpdate {
    Keep,
    Replace {
        #[ts(type = "string")]
        value: SecretValue,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AuthenticationMode {
    Shared,
    Separate,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PasswordAuthenticationInput {
    Shared {
        password: SecretUpdate,
    },
    Separate {
        imap_password: SecretUpdate,
        smtp_password: SecretUpdate,
    },
}

/// Reserved provider-verified identity boundary; not an enabled OAuth flow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OAuth2Configuration {
    pub provider_id: String,
    pub issuer: String,
    pub client_registration_id: String,
    pub authorization_subject: Option<String>,
    pub target_mailbox_resource: Option<String>,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AuthenticationInput {
    Password {
        #[serde(flatten)]
        credentials: PasswordAuthenticationInput,
    },
    #[serde(rename = "oauth2")]
    OAuth2 { configuration: OAuth2Configuration },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProxyKind {
    Socks5,
    HttpConnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DnsPolicy {
    Proxy,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyAuthenticationInput {
    pub username: String,
    pub password: SecretUpdate,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RouteInput {
    Direct,
    Inherit,
    Custom {
        proxy_kind: ProxyKind,
        host: String,
        port: u16,
        dns: DnsPolicy,
        authentication: Option<ProxyAuthenticationInput>,
    },
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountConfigInput {
    pub display_name: String,
    pub email_address: String,
    pub provider_id: Option<String>,
    pub imap: ConnectionConfig,
    pub smtp: ConnectionConfig,
    pub authentication: AuthenticationInput,
    pub route: RouteInput,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAccountRequest {
    pub request_id: String,
    pub config: AccountConfigInput,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAccountRequest {
    pub account_id: String,
    #[ts(type = "number")]
    pub expected_revision: u32,
    pub config: AccountConfigInput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CredentialTrust {
    Trusted,
    NeedsConfirmation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationSummary {
    pub id: String,
    #[ts(type = "number")]
    pub generation: u32,
    pub password_configured: bool,
    pub trust: CredentialTrust,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PasswordAuthenticationSummary {
    pub mode: AuthenticationMode,
    pub imap: AuthenticationSummary,
    pub smtp: AuthenticationSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProxyAuthenticationSummary {
    pub username: String,
    pub password_configured: bool,
    pub trust: CredentialTrust,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RouteConfig {
    Direct,
    Inherit,
    Custom {
        proxy_kind: ProxyKind,
        host: String,
        port: u16,
        dns: DnsPolicy,
        authentication: Option<ProxyAuthenticationSummary>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionStatus {
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    #[ts(type = "number")]
    pub revision: u32,
    pub display_name: String,
    pub email_address: String,
    pub provider_id: Option<String>,
    pub mailbox_source_id: String,
    pub imap: ConnectionConfig,
    pub smtp: ConnectionConfig,
    pub authentication: PasswordAuthenticationSummary,
    pub route: RouteConfig,
    #[ts(type = "number")]
    pub route_revision: u32,
    /// The initial application default is direct at revision 1.
    #[ts(type = "number | null")]
    pub default_route_revision: Option<u32>,
    pub connection_status: ConnectionStatus,
    pub created_at: String,
    pub updated_at: String,
}
