//! Desktop-independent authorization orchestration. Only reviewed registrations
//! and identity adapters can start a session; the caller never supplies URLs.
use super::*;
use crate::accounts::{
    oauth_sessions::SignInState,
    types::{AccountProfileInput, RouteInput},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Starts browser sign-in for a new account. There is no address yet: it is
/// learned from the sign-in and confirmed against the mail server. Servers and
/// login names come from the reviewed provider entry, never from the caller.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeginOAuthAccount {
    pub provider_id: String,
    /// How MailX reaches the provider; also the new account's route.
    pub route: RouteInput,
}
/// An address typed by the user after sign-in. It counts only once the mail
/// server opens that mailbox with the session's own token.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmOAuthMailbox {
    pub session_id: String,
    pub email_address: String,
}
/// Creates the account from a signed-in session. Only the profile comes from
/// the page; the address, servers, login names and route come from the session.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateOAuthAccount {
    pub session_id: String,
    pub request_id: String,
    pub profile: AccountProfileInput,
}
/// What the page learns once the browser has returned or an address was tried.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OAuthSignInResult {
    /// The mail server opened this mailbox with the new sign-in.
    Verified { email_address: String },
    /// No usable mailbox yet; ask for the address. `suggestion` is the address
    /// the sign-in offered, if any, which the mail server did not accept.
    NeedsMailbox { suggestion: Option<String> },
}
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeginOAuthReauthorization {
    pub provider_id: String,
    pub account_id: String,
    #[ts(type = "number")]
    pub expected_revision: u32,
}
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OAuthSessionRequest {
    pub session_id: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OAuthSessionHandle {
    pub session_id: String,
}
/// Trusted host-only result. The authorization URL contains transient state;
/// it must be opened directly by the host and never logged or persisted.
pub struct OAuthBrowserAuthorization {
    pub session_id: String,
    pub authorization_url: String,
}
fn error(error: crate::auth::OAuthError) -> AppError {
    use crate::auth::OAuthError;
    AppError::new(match error {
        OAuthError::UnsupportedProvider => ErrorCode::Unsupported,
        OAuthError::Storage => ErrorCode::StorageUnavailable,
        OAuthError::Network => ErrorCode::OAuthNetwork,
        OAuthError::Proxy => ErrorCode::OAuthProxy,
        OAuthError::Tls => ErrorCode::OAuthTls,
        OAuthError::RateLimited => ErrorCode::OAuthRateLimited,
        OAuthError::ServerUnavailable => ErrorCode::OAuthServerUnavailable,
        OAuthError::Timeout => ErrorCode::OAuthTimeout,
        OAuthError::AuthorizationDenied => ErrorCode::OAuthAuthorizationDenied,
        OAuthError::ExpiredSession => ErrorCode::OAuthSessionExpired,
        OAuthError::ReauthenticationRequired => ErrorCode::OAuthReauthorizationRequired,
        OAuthError::InvalidResponse | OAuthError::InvalidCallback => {
            ErrorCode::OAuthInvalidResponse
        }
        OAuthError::MailboxRefused => ErrorCode::OAuthMailboxRefused,
        OAuthError::UsePersonalEntry => ErrorCode::OAuthUsePersonalEntry,
        OAuthError::UseOrganizationEntry => ErrorCode::OAuthUseOrganizationEntry,
        OAuthError::InvalidRegistration => ErrorCode::Unsupported,
        OAuthError::Random => ErrorCode::Internal,
    })
}
/// Upper bound for quit-time OAuth settlement, after the host has committed
/// to exiting. Persistence retries take a few seconds at most when healthy.
const OAUTH_SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

impl From<SignInState> for OAuthSignInResult {
    fn from(state: SignInState) -> Self {
        match state {
            SignInState::Verified(email_address) => Self::Verified { email_address },
            SignInState::NeedsMailbox(suggestion) => Self::NeedsMailbox { suggestion },
        }
    }
}

impl MailxCore {
    pub async fn begin_oauth_account(
        &self,
        input: BeginOAuthAccount,
    ) -> Result<OAuthBrowserAuthorization, AppError> {
        // Lookup precedes validation, proxy credential access and loopback bind.
        // Empty production registries therefore cause no observable side effect.
        let provider = crate::providers::oauth::lookup(&input.provider_id).map_err(error)?;
        let registration = provider.registration().clone();
        let verifier = provider.identity_verifier();
        // The route is checked and resolved before the listener binds; a proxy
        // that fails never falls back to a direct connection.
        let route = crate::accounts::oauth::authorization_route(&input.route)?;
        let start = self
            .oauth_sessions
            .begin(registration, verifier, route, input.route)
            .await
            .map_err(error)?;
        Ok(OAuthBrowserAuthorization {
            session_id: start.session_id,
            authorization_url: start.authorization_url,
        })
    }
    pub async fn begin_oauth_reauthorization(
        &self,
        input: BeginOAuthReauthorization,
    ) -> Result<OAuthBrowserAuthorization, AppError> {
        let provider = crate::providers::oauth::lookup(&input.provider_id).map_err(error)?;
        let registration = provider.registration().clone();
        let verifier = provider.identity_verifier();
        let service = self.service.clone();
        let account_id = input.account_id.clone();
        let expected_revision = input.expected_revision;
        let (request, route) = tokio::task::spawn_blocking(move || {
            service
                .lock()
                .map_err(|_| AppError::new(ErrorCode::Internal))?
                .prepare_oauth_reauthorization(&account_id, expected_revision)
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
        if request.config.provider_id.as_deref() != Some(&registration.id) {
            return Err(AppError::validation("providerId"));
        }
        let start = self
            .oauth_sessions
            .begin_reauthorization(
                registration,
                verifier,
                route,
                request,
                input.account_id,
                input.expected_revision,
            )
            .await
            .map_err(error)?;
        Ok(OAuthBrowserAuthorization {
            session_id: start.session_id,
            authorization_url: start.authorization_url,
        })
    }
    /// Called after an irreversible host quit decision, while its async runtime
    /// is still alive. Stop new refresh users, then let owned refresh persistence
    /// settle before the host tears down its runtime. The whole step is bounded:
    /// a hung keychain write or prompt must not keep the app from exiting.
    pub async fn settle_oauth_authorizations(&self) -> Result<(), AppError> {
        tokio::time::timeout(OAUTH_SETTLE_LIMIT, self.settle_oauth_authorizations_inner())
            .await
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?
    }
    async fn settle_oauth_authorizations_inner(&self) -> Result<(), AppError> {
        self.oauth_sessions.shutdown();
        let mut stopped = self.outbox.shutdown();
        stopped.extend(self.sync.shutdown());
        stopped.extend(self.bodies.shutdown());
        stopped.extend(self.previews.shutdown());
        // Abort handles cannot be awaited; poll with a timer instead of
        // spinning so the coordinator thread does not burn a core.
        while stopped.iter().any(|task| !task.is_finished()) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let service = self.service.clone();
        let managers = tokio::task::spawn_blocking(move || {
            Ok::<_, AppError>(
                service
                    .lock()
                    .map_err(|_| AppError::new(ErrorCode::Internal))?
                    .oauth_managers_for_shutdown(),
            )
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
        let settled = futures_util::future::join_all(
            managers.iter().map(|manager| manager.settle_persistence()),
        )
        .await;
        if settled.iter().any(Result::is_err) {
            Err(AppError::new(ErrorCode::StorageUnavailable))
        } else {
            Ok(())
        }
    }
    pub fn cancel_oauth_account(&self, input: OAuthSessionRequest) -> Result<(), AppError> {
        self.oauth_sessions.cancel(&input.session_id).map_err(error)
    }
    /// Waits for the browser. The signed-in session is kept for a while so the
    /// page can ask for an address and create the account.
    pub async fn finish_oauth_account(
        &self,
        input: OAuthSessionRequest,
    ) -> Result<OAuthSignInResult, AppError> {
        self.oauth_sessions
            .finish(&input.session_id)
            .await
            .map(Into::into)
            .map_err(error)
    }
    /// Tries a typed address with the session's own token; no second browser trip.
    pub async fn confirm_oauth_mailbox(
        &self,
        input: ConfirmOAuthMailbox,
    ) -> Result<OAuthSignInResult, AppError> {
        self.oauth_sessions
            .confirm_mailbox(&input.session_id, &input.email_address)
            .await
            .map(Into::into)
            .map_err(error)
    }
    /// Creates the account from a session whose mailbox the mail server accepted.
    pub async fn create_oauth_account(
        &self,
        input: CreateOAuthAccount,
    ) -> Result<Account, AppError> {
        let target = self
            .oauth_sessions
            .begin_create(&input.session_id)
            .map_err(error)?;
        let mailbox = target.authorization.binding.mailbox.clone();
        let request = crate::accounts::oauth::create_request(
            &target.registration,
            &mailbox,
            target.route,
            input.request_id,
            input.profile,
        );
        let service = self.service.clone();
        let created = tokio::task::spawn_blocking(move || {
            service
                .lock()
                .map_err(|_| AppError::new(ErrorCode::Internal))?
                .create_oauth(request, target.authorization, target.registration)
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal));
        match created {
            Ok(Ok(account)) => {
                self.oauth_sessions.created(&input.session_id);
                self.sync.restart(&account.id);
                self.outbox.start(&account.id);
                Ok(account)
            }
            Ok(Err(failure)) | Err(failure) => {
                // Nothing was saved, so the user can fix the profile and retry.
                self.oauth_sessions.create_failed(&input.session_id);
                Err(failure)
            }
        }
    }
    /// Waits for the browser of a reauthorization and saves the renewed grant.
    pub async fn finish_oauth_reauthorization(
        &self,
        input: OAuthSessionRequest,
    ) -> Result<Account, AppError> {
        let completed = self
            .oauth_sessions
            .finish_reauthorization(&input.session_id)
            .await
            .map_err(error)?;
        let service = self.service.clone();
        let account = tokio::task::spawn_blocking(move || {
            service
                .lock()
                .map_err(|_| AppError::new(ErrorCode::Internal))?
                .reauthorize_oauth(
                    &completed.account_id,
                    completed.expected_revision,
                    completed.authorization,
                    completed.registration,
                )
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
        self.sync.restart(&account.id);
        self.outbox.start(&account.id);
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorization_failures_keep_actionable_sanitized_categories() {
        use crate::auth::OAuthError;
        assert_eq!(error(OAuthError::Network).code, ErrorCode::OAuthNetwork);
        assert_eq!(error(OAuthError::Proxy).code, ErrorCode::OAuthProxy);
        assert_eq!(error(OAuthError::Timeout).code, ErrorCode::OAuthTimeout);
        assert_eq!(
            error(OAuthError::AuthorizationDenied).code,
            ErrorCode::OAuthAuthorizationDenied
        );
        assert_eq!(
            error(OAuthError::ExpiredSession).code,
            ErrorCode::OAuthSessionExpired
        );
        assert_eq!(
            error(OAuthError::ReauthenticationRequired).code,
            ErrorCode::OAuthReauthorizationRequired
        );
        assert_eq!(
            error(OAuthError::InvalidCallback).code,
            ErrorCode::OAuthInvalidResponse
        );
        assert_eq!(
            error(OAuthError::MailboxRefused).code,
            ErrorCode::OAuthMailboxRefused
        );
        assert_eq!(
            error(OAuthError::UsePersonalEntry).code,
            ErrorCode::OAuthUsePersonalEntry
        );
        assert_eq!(
            error(OAuthError::UseOrganizationEntry).code,
            ErrorCode::OAuthUseOrganizationEntry
        );
    }
    #[tokio::test]
    async fn unknown_provider_begin_and_reauthorization_fail_before_account_access() {
        let directory = tempfile::TempDir::new().unwrap();
        let core = MailxCore::open(
            directory.path(),
            Arc::new(crate::secrets::test_support::MemorySecrets::default()),
            Handle::current(),
            Arc::new(|_: &CoreEvent| {}),
        )
        .unwrap();
        let error = match core
            .begin_oauth_account(BeginOAuthAccount {
                provider_id: "https://untrusted.example/authorize".into(),
                route: RouteInput::Direct,
            })
            .await
        {
            Ok(_) => panic!("unexpected OAuth session"),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::Unsupported);
        let error = match core
            .begin_oauth_reauthorization(BeginOAuthReauthorization {
                provider_id: "unknown".into(),
                account_id: "not-an-account".into(),
                expected_revision: 1,
            })
            .await
        {
            Ok(_) => panic!("unexpected OAuth session"),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert!(core.accounts().unwrap().list().unwrap().is_empty());
        core.settle_oauth_authorizations().await.unwrap();
    }
}
