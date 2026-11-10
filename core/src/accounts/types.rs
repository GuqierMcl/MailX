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

/// Sanitized OAuth identity metadata. Only backend-verified authorization
/// establishes these values; public configuration input is never proof of identity.
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
        /// HTTPS images may delegate target DNS to this configured proxy.
        #[serde(default)]
        image_proxy_dns: bool,
        /// Explicit grant for the endpoint supplied in this request. Never persisted.
        #[serde(default)]
        confirm_image_proxy_dns: bool,
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
    /// The name recipients see in `From`; `null` or empty sends with the bare
    /// address. Unlike `displayName` it is never shown only to the user.
    #[serde(default)]
    pub sender_name: Option<String>,
    /// Prefer known provider artwork when available; omitted defaults to true.
    #[serde(default = "default_use_provider_icon")]
    pub use_provider_icon: bool,
}

fn default_use_provider_icon() -> bool {
    true
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
    /// The name recipients see in `From`; omitted, `null` or empty for none.
    #[serde(default)]
    pub sender_name: Option<String>,
    /// Prefer known provider artwork when available; omitted defaults to true.
    #[serde(default = "default_use_provider_icon")]
    pub use_provider_icon: bool,
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
                sender_name: self.sender_name,
                use_provider_icon: self.use_provider_icon,
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

/// Removes only the account's local data; never deletes mail on its server.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveAccountRequest {
    pub account_id: String,
    #[ts(type = "number")]
    pub expected_revision: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AccountRemoval {
    pub account_id: String,
    /// Includes every deleted local draft so hosts can retire its editor.
    pub draft_ids: Vec<String>,
    /// Local removal committed; host-owned credentials will be retried later.
    pub credentials_pending: bool,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasswordAuthenticationSummary {
    pub mode: AuthenticationMode,
    pub imap: AuthenticationSummary,
    pub smtp: AuthenticationSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolAuthenticationSummary {
    pub id: String,
    #[ts(type = "number")]
    pub generation: u32,
    pub credential_configured: bool,
    pub trust: CredentialTrust,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum OAuthAuthenticationState {
    Connected,
    NeedsReauthorization,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OAuthAuthenticationSummary {
    pub configuration: OAuth2Configuration,
    pub state: OAuthAuthenticationState,
    pub imap: ProtocolAuthenticationSummary,
    pub smtp: ProtocolAuthenticationSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AccountAuthenticationSummary {
    Password(PasswordAuthenticationSummary),
    #[serde(rename = "oauth2")]
    OAuth2(OAuthAuthenticationSummary),
}

impl<'de> Deserialize<'de> for AccountAuthenticationSummary {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "camelCase")]
        enum Tagged {
            Password(PasswordAuthenticationSummary),
            #[serde(rename = "oauth2")]
            OAuth2(OAuthAuthenticationSummary),
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Compatible {
            Tagged(Tagged),
            Legacy(PasswordAuthenticationSummary),
        }
        match Compatible::deserialize(deserializer)? {
            Compatible::Tagged(Tagged::Password(value)) | Compatible::Legacy(value) => {
                Ok(Self::Password(value))
            }
            Compatible::Tagged(Tagged::OAuth2(value)) => Ok(Self::OAuth2(value)),
        }
    }
}

impl AccountAuthenticationSummary {
    pub fn is_oauth(&self) -> bool {
        matches!(self, Self::OAuth2(_))
    }
    pub fn password(&self) -> Option<&PasswordAuthenticationSummary> {
        match self {
            Self::Password(value) => Some(value),
            Self::OAuth2(_) => None,
        }
    }
    pub fn imap(&self) -> ProtocolAuthenticationSummary {
        match self {
            Self::Password(value) => ProtocolAuthenticationSummary::from(&value.imap),
            Self::OAuth2(value) => value.imap.clone(),
        }
    }
    pub fn smtp(&self) -> ProtocolAuthenticationSummary {
        match self {
            Self::Password(value) => ProtocolAuthenticationSummary::from(&value.smtp),
            Self::OAuth2(value) => value.smtp.clone(),
        }
    }
}
impl From<&AuthenticationSummary> for ProtocolAuthenticationSummary {
    fn from(value: &AuthenticationSummary) -> Self {
        Self {
            id: value.id.clone(),
            generation: value.generation,
            credential_configured: value.password_configured,
            trust: value.trust,
        }
    }
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
        /// Defaults closed when reading snapshots written before this policy existed.
        #[serde(default)]
        image_proxy_dns: bool,
        authentication: Option<ProxyAuthenticationSummary>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionStatus {
    Unverified,
}

/// Local account labels without connection or credential information.
pub(crate) struct AccountIdentity {
    pub id: String,
    pub display_name: String,
    pub email_address: String,
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
    /// The name recipients see in `From`, or `null` to send the bare address.
    #[serde(default)]
    pub sender_name: Option<String>,
    /// Appearance only. Defaults to true for credential journals from older builds.
    #[serde(default = "default_use_provider_icon")]
    pub use_provider_icon: bool,
    pub email_address: String,
    pub provider_id: Option<String>,
    pub mailbox_source_id: String,
    pub imap: ConnectionConfig,
    pub smtp: ConnectionConfig,
    pub authentication: AccountAuthenticationSummary,
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

#[cfg(test)]
mod authentication_contract_tests {
    use super::*;
    #[test]
    fn legacy_password_summary_reads_and_new_summary_is_tagged() {
        let legacy = serde_json::json!({"mode":"shared", "imap":{"id":"a","generation":1,"passwordConfigured":true,"trust":"trusted"}, "smtp":{"id":"a","generation":1,"passwordConfigured":true,"trust":"trusted"}});
        let summary: AccountAuthenticationSummary = serde_json::from_value(legacy.clone()).unwrap();
        assert!(!summary.is_oauth());
        assert_eq!(serde_json::to_value(summary).unwrap()["kind"], "password");
        let mut unknown = legacy;
        unknown["kind"] = serde_json::json!("unknown");
        assert!(serde_json::from_value::<AccountAuthenticationSummary>(unknown).is_err());
    }
    #[test]
    fn oauth_summary_exposes_no_password_or_tokens() {
        let endpoint = ProtocolAuthenticationSummary {
            id: "opaque-id".into(),
            generation: 1,
            credential_configured: true,
            trust: CredentialTrust::Trusted,
        };
        let summary = AccountAuthenticationSummary::OAuth2(OAuthAuthenticationSummary {
            configuration: OAuth2Configuration {
                provider_id: "reviewed-provider".into(),
                issuer: "https://issuer.example".into(),
                client_registration_id: "registration".into(),
                authorization_subject: Some("subject".into()),
                target_mailbox_resource: None,
            },
            state: OAuthAuthenticationState::Connected,
            imap: endpoint.clone(),
            smtp: endpoint,
        });
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("password"));
        assert!(!json.contains("token"));
        assert_eq!(
            serde_json::from_str::<AccountAuthenticationSummary>(&json).unwrap(),
            summary
        );
    }
}
