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

/// How the account appears in MailX. Changing it never touches a connection.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountProfileInput {
    pub display_name: String,
    /// `#rrggbb`, or `null` for the neutral default.
    pub color: Option<String>,
}

/// How MailX reaches the mailbox: identity, servers, credentials and route.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionInput {
    pub email_address: String,
    pub provider_id: Option<String>,
    pub imap: ConnectionConfig,
    pub smtp: ConnectionConfig,
    pub authentication: AuthenticationInput,
    pub route: RouteInput,
}

/// A new account: its profile and its connection in one request.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountConfigInput {
    pub display_name: String,
    /// `#rrggbb`; omitted or `null` for the neutral default.
    #[serde(default)]
    pub color: Option<String>,
    pub email_address: String,
    pub provider_id: Option<String>,
    pub imap: ConnectionConfig,
    pub smtp: ConnectionConfig,
    pub authentication: AuthenticationInput,
    pub route: RouteInput,
}

impl AccountConfigInput {
    pub(crate) fn split(self) -> (AccountProfileInput, ConnectionInput) {
        (
            AccountProfileInput {
                display_name: self.display_name,
                color: self.color,
            },
            ConnectionInput {
                email_address: self.email_address,
                provider_id: self.provider_id,
                imap: self.imap,
                smtp: self.smtp,
                authentication: self.authentication,
                route: self.route,
            },
        )
    }
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAccountRequest {
    pub request_id: String,
    pub config: AccountConfigInput,
}

/// Replaces the connection. Advances `revision` and restarts sync.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateConnectionRequest {
    pub account_id: String,
    #[ts(type = "number")]
    pub expected_revision: u32,
    pub connection: ConnectionInput,
}

/// Replaces the profile. Leaves `revision` and any running sync untouched;
/// the last write wins.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateProfileRequest {
    pub account_id: String,
    pub profile: AccountProfileInput,
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
    /// Version of the connection settings. Profile changes do not advance it.
    #[ts(type = "number")]
    pub revision: u32,
    pub display_name: String,
    /// `#rrggbb`, or `null` for the neutral default. Defaulted so credential
    /// journals written before colors existed still load.
    #[serde(default)]
    pub color: Option<String>,
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
