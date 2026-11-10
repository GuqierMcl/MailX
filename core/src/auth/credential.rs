use crate::secrets::SecretValue;

/// Credentials for a protocol connection, independent of any OAuth provider.
#[derive(Clone, Debug)]
pub(crate) enum ProtocolCredential {
    Password(SecretValue),
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Raw tokens are only constructed by protocol fixtures; production resolves managed grants"
        )
    )]
    OAuth2(SecretValue),
    OAuth2Guarded(SecretValue, std::sync::Arc<super::TokenManager>),
    ManagedOAuth2(std::sync::Arc<super::TokenManager>),
}
impl ProtocolCredential {
    pub(crate) async fn resolve(
        &self,
        route: &crate::net::NetRoute,
    ) -> Result<Self, super::OAuthError> {
        match self {
            Self::ManagedOAuth2(manager) => Ok(Self::OAuth2Guarded(
                manager.access_token(route).await?,
                manager.clone(),
            )),
            other => Ok(other.clone()),
        }
    }
    pub(crate) fn secret(&self) -> Result<&SecretValue, super::OAuthError> {
        match self {
            Self::Password(secret) | Self::OAuth2(secret) => Ok(secret),
            Self::OAuth2Guarded(secret, manager)
                if manager.is_valid() && !manager.needs_reauthorization() =>
            {
                Ok(secret)
            }
            Self::OAuth2Guarded(_, _) => Err(super::OAuthError::ReauthenticationRequired),
            Self::ManagedOAuth2(_) => Err(super::OAuthError::InvalidResponse),
        }
    }
    /// The mail server refused this resolved bearer. Returns whether resolving
    /// the original managed credential again can produce a different token.
    pub(crate) fn reject(&self) -> bool {
        match self {
            Self::OAuth2Guarded(secret, manager) => {
                manager.reject_access_token(secret);
                manager.is_valid() && !manager.needs_reauthorization()
            }
            _ => false,
        }
    }
    /// Lets the provider recognize an administrator-disabled protocol. A
    /// recognized rejection says nothing about the grant, so callers must not
    /// refresh, retry or mark it for reauthorization.
    pub(crate) fn classify_rejection(
        &self,
        rejection: &super::MailRejection<'_>,
    ) -> Option<super::RejectionKind> {
        match self {
            Self::OAuth2Guarded(_, manager) | Self::ManagedOAuth2(manager) => {
                manager.classify_rejection(rejection)
            }
            _ => None,
        }
    }
    pub(crate) fn is_oauth(&self) -> bool {
        matches!(
            self,
            Self::OAuth2(_) | Self::OAuth2Guarded(_, _) | Self::ManagedOAuth2(_)
        )
    }
}
