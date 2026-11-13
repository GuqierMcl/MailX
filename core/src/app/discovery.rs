//! Mail server discovery for the unsaved add-account form.
//!
//! The lookup is read-only: it never creates an account, never touches the
//! credential store, and never changes a saved connection. It only reports
//! servers the form may fill in, so a user can still type them by hand when
//! discovery finds nothing.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::MailxCore;
use crate::accounts::draft_connection::route_from_input;
use crate::accounts::normalize_email_address;
use crate::accounts::types::{EwsAuth, RouteInput, TlsMode};
use crate::discovery::{self, Credentials, DiscoveredEndpoint, DiscoveryError, EwsEndpoint};
use crate::error::AppError;
use crate::secrets::SecretValue;

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoverMailSettingsRequest {
    /// The mailbox to look up. Its domain decides which hosts may be contacted.
    pub email_address: String,
    /// Sent only to the Autodiscover endpoint of that domain, and only when the
    /// form has a password to offer. An unauthenticated lookup still works on
    /// servers that publish settings without credentials.
    #[serde(default)]
    #[ts(type = "string | null")]
    pub password: Option<SecretValue>,
    /// Follows the same proxy the form selected; never falls back to direct.
    #[serde(default = "direct_route")]
    pub route: RouteInput,
}

fn direct_route() -> RouteInput {
    RouteInput::Direct
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MailSettingsDiscoveryStatus {
    Found,
    /// The endpoint asked for authentication and the request carried none.
    NeedsCredentials,
    /// The supplied credentials were rejected, or the mailbox reported an error.
    Rejected,
    NotFound,
    Network,
    Proxy,
    Tls,
    Timeout,
    /// The answer was unusable: not HTTPS, no TLS endpoint, or malformed.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredMailEndpoint {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    /// `null` leaves the form's default (the mailbox address) untouched.
    pub username: Option<String>,
}

/// An on-premises Exchange endpoint reached over HTTPS with NTLM. This is the
/// "use HTTP to connect to Microsoft Exchange" configuration: one URL handles
/// both reading and sending, so no IMAP or SMTP host is required.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredEwsEndpoint {
    pub url: String,
    pub auth: EwsAuth,
    pub username: Option<String>,
    pub cert_principal_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MailSettingsDiscovery {
    pub status: MailSettingsDiscoveryStatus,
    pub imap: Option<DiscoveredMailEndpoint>,
    pub smtp: Option<DiscoveredMailEndpoint>,
    pub ews: Option<DiscoveredEwsEndpoint>,
    /// Origin that answered, so the user can see where the values came from.
    pub source: Option<String>,
}

impl MailxCore {
    /// Looks up settings for an address the user has typed but not saved.
    /// Nothing here becomes a stored trust decision: saving the form still
    /// records credentials against the endpoints it submits.
    pub async fn discover_mail_settings(
        &self,
        request: DiscoverMailSettingsRequest,
    ) -> Result<MailSettingsDiscovery, AppError> {
        let email = normalize_email_address(&request.email_address)?;
        let route = route_from_input(request.route)?;
        let credentials = request.password.as_ref().map(|password| Credentials {
            username: &email,
            password,
        });
        let outcome = discovery::discover(&route, &email, credentials.as_ref()).await;
        Ok(match outcome {
            Ok(settings) => {
                let found =
                    settings.imap.is_some() || settings.smtp.is_some() || settings.ews.is_some();
                MailSettingsDiscovery {
                    status: if found {
                        MailSettingsDiscoveryStatus::Found
                    } else {
                        MailSettingsDiscoveryStatus::NotFound
                    },
                    imap: settings.imap.map(endpoint),
                    smtp: settings.smtp.map(endpoint),
                    ews: settings.ews.map(ews_endpoint),
                    // Only a useful answer names where it came from.
                    source: if found { settings.source } else { None },
                }
            }
            Err(error) => MailSettingsDiscovery {
                status: status_of(error),
                imap: None,
                smtp: None,
                ews: None,
                source: None,
            },
        })
    }
}

fn endpoint(found: DiscoveredEndpoint) -> DiscoveredMailEndpoint {
    DiscoveredMailEndpoint {
        host: found.host,
        port: found.port,
        tls: found.tls,
        username: found.username,
    }
}

fn ews_endpoint(found: EwsEndpoint) -> DiscoveredEwsEndpoint {
    DiscoveredEwsEndpoint {
        url: found.url,
        auth: match found.auth {
            discovery::EwsAuth::Ntlm => EwsAuth::Ntlm,
            discovery::EwsAuth::Basic => EwsAuth::Basic,
        },
        username: found.username,
        cert_principal_name: found.cert_principal_name,
    }
}

fn status_of(error: DiscoveryError) -> MailSettingsDiscoveryStatus {
    match error {
        DiscoveryError::NeedsCredentials => MailSettingsDiscoveryStatus::NeedsCredentials,
        DiscoveryError::Rejected => MailSettingsDiscoveryStatus::Rejected,
        DiscoveryError::NotFound => MailSettingsDiscoveryStatus::NotFound,
        DiscoveryError::Network => MailSettingsDiscoveryStatus::Network,
        DiscoveryError::Proxy => MailSettingsDiscoveryStatus::Proxy,
        DiscoveryError::Tls => MailSettingsDiscoveryStatus::Tls,
        DiscoveryError::Timeout => MailSettingsDiscoveryStatus::Timeout,
        DiscoveryError::Unsupported => MailSettingsDiscoveryStatus::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_covers_every_discovery_failure() {
        for error in [
            DiscoveryError::NeedsCredentials,
            DiscoveryError::Rejected,
            DiscoveryError::NotFound,
            DiscoveryError::Network,
            DiscoveryError::Proxy,
            DiscoveryError::Tls,
            DiscoveryError::Timeout,
            DiscoveryError::Unsupported,
        ] {
            let status = status_of(error);
            assert_ne!(status, MailSettingsDiscoveryStatus::Found);
        }
    }

    #[test]
    fn ews_endpoint_keeps_its_auth_and_certificate_name() {
        let found = EwsEndpoint {
            url: "https://mail.example.com/EWS/Exchange.asmx".into(),
            auth: discovery::EwsAuth::Ntlm,
            username: Some("EXAMPLE\\alice".into()),
            cert_principal_name: Some("msstd:mail.example.com".into()),
        };
        let result = ews_endpoint(found);
        assert_eq!(result.auth, EwsAuth::Ntlm);
        assert_eq!(result.username.as_deref(), Some("EXAMPLE\\alice"));
        assert_eq!(
            result.cert_principal_name.as_deref(),
            Some("msstd:mail.example.com")
        );
    }
}
