//! Desktop-independent authorization orchestration. Only reviewed registrations
//! and identity adapters can start a session; the caller never supplies URLs.
use super::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeginOAuthAccount {
    pub provider_id: String,
    pub request: CreateAccountRequest,
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
        OAuthError::InvalidRegistration => ErrorCode::Unsupported,
        OAuthError::Random => ErrorCode::Internal,
    })
}
impl MailxCore {
    pub async fn begin_oauth_account(
        &self,
        input: BeginOAuthAccount,
    ) -> Result<OAuthBrowserAuthorization, AppError> {
        // Lookup precedes validation, proxy credential access and loopback bind.
        // Empty production registries therefore cause no observable side effect.
        let registration = crate::auth::lookup_registration(&input.provider_id).map_err(error)?;
        let verifier = crate::auth::lookup_identity_verifier(&input.provider_id).map_err(error)?;
        let mut request = input.request;
        // Public identity hints are never proof of an OAuth grant. The account
        // target uses only trusted registration metadata and verified identity.
        request.config.provider_id = Some(registration.id.clone());
        request.config.authentication = crate::accounts::types::AuthenticationInput::OAuth2 {
            configuration: crate::accounts::types::OAuth2Configuration {
                provider_id: registration.id.clone(),
                issuer: registration.issuer.clone(),
                client_registration_id: registration.registration_id.clone(),
                authorization_subject: None,
                target_mailbox_resource: Some(request.config.email_address.clone()),
            },
        };
        let route = crate::accounts::oauth::authorization_route(&request)?;
        let start = self
            .oauth_sessions
            .begin(registration, verifier, route, request)
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
        let registration = crate::auth::lookup_registration(&input.provider_id).map_err(error)?;
        let verifier = crate::auth::lookup_identity_verifier(&input.provider_id).map_err(error)?;
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
    /// settle before the host tears down its runtime.
    pub async fn settle_oauth_authorizations(&self) -> Result<(), AppError> {
        self.oauth_sessions.shutdown();
        let mut stopped = self.outbox.shutdown();
        stopped.extend(self.sync.shutdown());
        stopped.extend(self.bodies.shutdown());
        stopped.extend(self.previews.shutdown());
        while stopped.iter().any(|task| !task.is_finished()) {
            tokio::task::yield_now().await;
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
        let mut failed = false;
        for manager in managers {
            failed |= manager.settle_persistence().await.is_err();
        }
        if failed {
            Err(AppError::new(ErrorCode::StorageUnavailable))
        } else {
            Ok(())
        }
    }
    pub fn cancel_oauth_account(&self, input: OAuthSessionRequest) -> Result<(), AppError> {
        self.oauth_sessions.cancel(&input.session_id).map_err(error)
    }
    pub async fn finish_oauth_account(
        &self,
        input: OAuthSessionRequest,
    ) -> Result<Account, AppError> {
        let completed = self
            .oauth_sessions
            .finish(&input.session_id)
            .await
            .map_err(error)?;
        let service = self.service.clone();
        let account = tokio::task::spawn_blocking(move || {
            let mut service = service
                .lock()
                .map_err(|_| AppError::new(ErrorCode::Internal))?;
            match completed.reauthorization {
                Some((account_id, expected_revision)) => service.reauthorize_oauth(
                    &account_id,
                    expected_revision,
                    completed.authorization,
                    completed.registration,
                ),
                None => service.create_oauth(
                    completed.request,
                    completed.authorization,
                    completed.registration,
                ),
            }
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
        let request: CreateAccountRequest = serde_json::from_value(serde_json::json!({
            "requestId": uuid::Uuid::new_v4().to_string(),
            "config": crate::accounts::sync_plan::tests::config(serde_json::json!({"kind":"direct"}))
        })).unwrap();
        let error = match core
            .begin_oauth_account(BeginOAuthAccount {
                provider_id: "https://untrusted.example/authorize".into(),
                request,
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
