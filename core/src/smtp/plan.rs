//! What a send attempt needs from an account: login, route and sender, built
//! only after the stored password is confirmed for this exact SMTP endpoint
//! (`AccountService::smtp_plan`, next to the IMAP `sync_plan`).

use super::{FailureReason, PreDataFailure, SmtpLogin, SmtpOutcome};
use crate::accounts::sync_plan::PlanError;
use crate::error::ErrorCode;
use crate::net::NetRoute;

#[derive(Debug)]
pub(crate) struct SmtpPlan {
    pub login: SmtpLogin,
    pub route: NetRoute,
    /// The account address, used for `MAIL FROM` and `From`.
    pub sender_address: String,
    pub sender_name: Option<String>,
}

/// A plan that could not be built is a failure before any connection:
/// permanent when the user must act, transient when the secret store may recover.
pub(crate) fn plan_failure(error: &PlanError) -> SmtpOutcome {
    let (permanent, reason) = match error {
        PlanError::CredentialUntrusted => (true, FailureReason::CredentialUntrusted),
        PlanError::CredentialMissing => (true, FailureReason::CredentialMissing),
        PlanError::Account(error) => match error.code {
            ErrorCode::SecretStoreLocked | ErrorCode::SecretStoreUnavailable => {
                (false, FailureReason::SecretStore)
            }
            _ => (true, FailureReason::Account),
        },
    };
    let failure = PreDataFailure::new(reason);
    if permanent {
        SmtpOutcome::Permanent(failure)
    } else {
        SmtpOutcome::Transient(failure)
    }
}
