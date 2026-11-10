//! Microsoft sign-in for Outlook.com and Microsoft 365 mailboxes. Every
//! Microsoft-specific fact (endpoints, servers, claims, error codes) lives here;
//! `crate::auth`, sync and send only see the neutral traits.
//!
//! Evaluation, registration and open questions:
//! `docs/reference/microsoft-integration.md`.
use crate::{
    accounts::types::TlsMode,
    auth::{
        CheckedIdentity, IdentityEvidence, IdentityVerifier, MailEndpointPolicy, MailLoginPolicy,
        MailProtocol, MailProtocolPolicy, MailRejection, OAuthError, OAuthProvider,
        ProviderRegistration, RejectionKind, address_shaped,
    },
    net::NetRoute,
    secrets::SecretValue,
};
use std::{future::Future, pin::Pin, sync::Arc, sync::LazyLock};
use url::Url;

/// Public client registered in Microsoft Entra; not a secret.
const CLIENT_ID: &str = "44a7bc3e-d9fd-4af9-90a6-a03b62013b40";
/// `tid` Microsoft gives every personal (consumer) account.
const CONSUMER_TENANT: &str = "9188040d-6c67-4c5b-b112-36a304b66dad";
const IMAP_HOST: &str = "outlook.office365.com";
const SCOPES: [&str; 6] = [
    "offline_access",
    "openid",
    "profile",
    "email",
    "https://outlook.office.com/IMAP.AccessAsUser.All",
    "https://outlook.office.com/SMTP.Send",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Audience {
    /// Outlook.com, Hotmail and Live accounts.
    Personal,
    /// Microsoft 365 work or school accounts.
    Organization,
}

/// Proves that a mailbox opens with a fresh bearer. Production signs in to the
/// reviewed IMAP server; tests substitute a script.
trait MailboxProof: Send + Sync {
    fn prove<'a>(
        &'a self,
        route: &'a NetRoute,
        imap: &'a MailProtocolPolicy,
        mailbox: &'a str,
        bearer: &'a SecretValue,
    ) -> Pin<Box<dyn Future<Output = Result<(), OAuthError>> + Send + 'a>>;
}
struct ImapProof;
impl MailboxProof for ImapProof {
    fn prove<'a>(
        &'a self,
        route: &'a NetRoute,
        imap: &'a MailProtocolPolicy,
        mailbox: &'a str,
        bearer: &'a SecretValue,
    ) -> Pin<Box<dyn Future<Output = Result<(), OAuthError>> + Send + 'a>> {
        Box::pin(crate::imap::prove_mailbox_access(
            route, &imap.host, imap.port, imap.tls, mailbox, bearer,
        ))
    }
}

pub(crate) struct MicrosoftProvider {
    registration: ProviderRegistration,
    audience: Audience,
}

fn registration(audience: Audience) -> ProviderRegistration {
    let (id, tenant, smtp_host) = match audience {
        Audience::Personal => (
            "microsoft-personal",
            "consumers",
            // Microsoft's Outlook.com settings page.
            "smtp-mail.outlook.com",
        ),
        Audience::Organization => ("microsoft-work", "organizations", "smtp.office365.com"),
    };
    let endpoint = |name: &str| {
        Url::parse(&format!(
            "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/{name}"
        ))
        .expect("static Microsoft endpoint")
    };
    ProviderRegistration {
        id: id.into(),
        client_id: CLIENT_ID.into(),
        registration_id: format!("{id}-v1"),
        mail_policy: MailEndpointPolicy {
            imap: MailProtocolPolicy {
                host: IMAP_HOST.into(),
                port: 993,
                tls: TlsMode::Implicit,
                login: MailLoginPolicy::Mailbox,
            },
            smtp: MailProtocolPolicy {
                host: smtp_host.into(),
                port: 587,
                tls: TlsMode::StartTls,
                login: MailLoginPolicy::Mailbox,
            },
        },
        authorization_endpoint: endpoint("authorize"),
        token_endpoint: endpoint("token"),
        // One token serves IMAP and SMTP; Microsoft takes no `resource`.
        scopes: SCOPES.iter().map(|scope| (*scope).to_owned()).collect(),
        resource: None,
        allowed_targets: vec![],
        // Registered as `http://localhost` / `http://127.0.0.1`, without a path.
        redirect_path: String::new(),
        extra_authorize_params: vec![("prompt".into(), "select_account".into())],
        login_hint: true,
    }
}

impl MicrosoftProvider {
    fn new(audience: Audience) -> Self {
        Self {
            registration: registration(audience),
            audience,
        }
    }
}
impl OAuthProvider for MicrosoftProvider {
    fn registration(&self) -> &ProviderRegistration {
        &self.registration
    }
    fn identity_verifier(&self) -> Arc<dyn IdentityVerifier> {
        Arc::new(MicrosoftVerifier {
            audience: self.audience,
            imap: self.registration.mail_policy.imap.clone(),
            proof: Arc::new(ImapProof),
        })
    }
    fn classify_rejection(&self, rejection: &MailRejection<'_>) -> Option<RejectionKind> {
        is_smtp_auth_disabled(rejection).then_some(RejectionKind::ProtocolDisabledByAdministrator)
    }
}

/// An organization turned SMTP AUTH off: `535 5.7.139`. IMAP only answers a
/// generic `NO AUTHENTICATE failed`, so it is never classified.
fn is_smtp_auth_disabled(rejection: &MailRejection<'_>) -> bool {
    rejection.protocol == MailProtocol::Smtp
        && rejection.smtp_code == Some(535)
        && rejection.enhanced_status == Some((5, 7, 139))
}

static PERSONAL: LazyLock<MicrosoftProvider> =
    LazyLock::new(|| MicrosoftProvider::new(Audience::Personal));
static ORGANIZATION: LazyLock<MicrosoftProvider> =
    LazyLock::new(|| MicrosoftProvider::new(Audience::Organization));

/// The Microsoft entries for the reviewed provider list.
pub(crate) fn providers() -> [&'static dyn OAuthProvider; 2] {
    [&*PERSONAL, &*ORGANIZATION]
}

struct MicrosoftVerifier {
    audience: Audience,
    imap: MailProtocolPolicy,
    proof: Arc<dyn MailboxProof>,
}
impl IdentityVerifier for MicrosoftVerifier {
    fn check<'a>(
        &'a self,
        evidence: &'a IdentityEvidence<'a>,
        _route: &'a NetRoute,
    ) -> Pin<Box<dyn Future<Output = Result<CheckedIdentity, OAuthError>> + Send + 'a>> {
        Box::pin(async move {
            let claims = evidence.id_token_claims()?;
            let tenant = claims.string("tid").ok_or(OAuthError::InvalidResponse)?;
            let object = claims.string("oid").ok_or(OAuthError::InvalidResponse)?;
            if !is_guid(tenant)
                || !is_guid(object)
                || claims.issuer != format!("https://login.microsoftonline.com/{tenant}/v2.0")
            {
                return Err(OAuthError::InvalidResponse);
            }
            let consumer = tenant.eq_ignore_ascii_case(CONSUMER_TENANT);
            match (self.audience, consumer) {
                (Audience::Personal, false) => return Err(OAuthError::UseOrganizationEntry),
                (Audience::Organization, true) => return Err(OAuthError::UsePersonalEntry),
                _ => {}
            }
            // `email` and `preferred_username` are mutable hints. They only
            // suggest which mailbox to try; the reviewed IMAP server decides.
            let hint = claims
                .string("email")
                .and_then(address_shaped)
                .or_else(|| claims.string("preferred_username").and_then(address_shaped));
            Ok(CheckedIdentity {
                issuer: claims.issuer.clone(),
                subject: format!(
                    "{}/{}",
                    tenant.to_ascii_lowercase(),
                    object.to_ascii_lowercase()
                ),
                mailbox_hint: hint,
            })
        })
    }
    fn prove_mailbox<'a>(
        &'a self,
        route: &'a NetRoute,
        bearer: &'a SecretValue,
        mailbox: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), OAuthError>> + Send + 'a>> {
        self.proof.prove(route, &self.imap, mailbox, bearer)
    }
}

/// The canonical hyphenated form only; `Uuid::parse_str` alone also accepts
/// braced, URN and simple spellings.
fn is_guid(text: &str) -> bool {
    text.len() == 36 && uuid::Uuid::parse_str(text).is_ok()
}

#[cfg(test)]
mod tests;
