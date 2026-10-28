//! Bounded, read-only diagnostics for an explicitly selected saved connection.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use ts_rs::TS;

use super::MailxCore;
use crate::accounts::{sync_plan::PlanError, validate_uuid};
use crate::error::{AppError, ErrorCode};
use crate::imap::{self, ImapError};
use crate::net::NetError;
use crate::smtp::{self, FailureReason, SmtpOutcome};

const BUDGET: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyAccountConnection {
    pub account_id: String,
    pub expected_revision: u32,
    /// Unique per attempt, so closing an old view cannot cancel a newer attempt.
    pub verification_id: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelConnectionVerification {
    pub account_id: String,
    pub verification_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionVerificationStatus {
    Verified,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionVerificationStage {
    Configuration,
    Connection,
    Tls,
    Authentication,
    Mailbox,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionVerificationCode {
    CredentialUntrusted,
    CredentialMissing,
    SecretStore,
    Network,
    Proxy,
    Tls,
    Timeout,
    Authentication,
    ProviderPolicy,
    AuthUnavailable,
    Protocol,
    Cancelled,
    Unsupported,
    Configuration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolVerification {
    pub status: ConnectionVerificationStatus,
    pub stage: ConnectionVerificationStage,
    pub code: Option<ConnectionVerificationCode>,
}

impl ProtocolVerification {
    fn verified() -> Self {
        Self {
            status: ConnectionVerificationStatus::Verified,
            stage: ConnectionVerificationStage::Complete,
            code: None,
        }
    }
    fn failed(stage: ConnectionVerificationStage, code: ConnectionVerificationCode) -> Self {
        Self {
            status: ConnectionVerificationStatus::Failed,
            stage,
            code: Some(code),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AccountConnectionVerification {
    pub account_id: String,
    pub revision: u32,
    pub imap: ProtocolVerification,
    pub smtp: ProtocolVerification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerificationControl {
    Active,
    Cancelled,
    Invalidated,
    TimedOut,
}

struct Flight {
    id: String,
    cancel: watch::Sender<VerificationControl>,
}
#[derive(Default)]
struct RegistryState {
    flights: HashMap<String, Flight>,
    // IPC scheduling may deliver cancellation before verification registration.
    // Keep only a bounded, short-lived set of these cancellation tombstones.
    cancelled: VecDeque<(String, String, Instant)>,
}
#[derive(Default)]
pub(super) struct VerificationRegistry(Mutex<RegistryState>);

struct Registration {
    registry: Arc<VerificationRegistry>,
    account: String,
    id: String,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut state) = self.registry.0.lock()
            && state
                .flights
                .get(&self.account)
                .is_some_and(|flight| flight.id == self.id)
        {
            state.flights.remove(&self.account);
        }
    }
}
impl VerificationRegistry {
    fn start(
        self: &Arc<Self>,
        account: &str,
        id: &str,
    ) -> Result<(Registration, watch::Receiver<VerificationControl>), AppError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        if state.flights.contains_key(account) {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        state
            .cancelled
            .retain(|(_, _, at)| at.elapsed() < BUDGET * 2);
        let cancelled = state
            .cancelled
            .iter()
            .position(|(a, i, _)| a == account && i == id)
            .map(|index| {
                state.cancelled.remove(index);
            })
            .is_some();
        let (cancel, receiver) = watch::channel(if cancelled {
            VerificationControl::Cancelled
        } else {
            VerificationControl::Active
        });
        state.flights.insert(
            account.to_owned(),
            Flight {
                id: id.to_owned(),
                cancel,
            },
        );
        Ok((
            Registration {
                registry: self.clone(),
                account: account.to_owned(),
                id: id.to_owned(),
            },
            receiver,
        ))
    }
    fn cancel(&self, account: &str, id: &str) -> Result<(), AppError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        if let Some(flight) = state.flights.get(account).filter(|flight| flight.id == id) {
            // A later view abort must not hide a configuration change.
            flight.cancel.send_if_modified(|control| {
                if *control == VerificationControl::Active {
                    *control = VerificationControl::Cancelled;
                    true
                } else {
                    false
                }
            });
        } else {
            state
                .cancelled
                .retain(|(_, _, at)| at.elapsed() < BUDGET * 2);
            if state.cancelled.len() >= 64 {
                state.cancelled.pop_front();
            }
            state
                .cancelled
                .push_back((account.to_owned(), id.to_owned(), Instant::now()));
        }
        Ok(())
    }
    pub(super) fn invalidate(&self, account: &str) {
        if let Ok(state) = self.0.lock()
            && let Some(flight) = state.flights.get(account)
        {
            flight.cancel.send_replace(VerificationControl::Invalidated);
        }
    }
}

impl MailxCore {
    /// Checks a saved version without fetching mail, changing flags, or sending.
    pub async fn verify_account_connection(
        &self,
        request: VerifyAccountConnection,
    ) -> Result<AccountConnectionVerification, AppError> {
        validate_uuid(&request.account_id, "accountId")?;
        validate_uuid(&request.verification_id, "verificationId")?;
        let (_registration, cancelled) = self
            .verification
            .start(&request.account_id, &request.verification_id)?;
        let deadline = tokio::time::Instant::now() + BUDGET;
        let revision = request.expected_revision;
        // One budget covers store/credential preparation, both network probes,
        // and final version validation. Dropping this future closes its sockets;
        // blocking store work also checks the control signal after taking a lock.
        let mut result = bounded(deadline, cancelled.clone(), async {
            let service = self.service.clone();
            let account_id = request.account_id.clone();
            let prepare_control = cancelled.clone();
            let plans = tokio::task::spawn_blocking(move || {
                let service = service
                    .lock()
                    .map_err(|_| AppError::new(ErrorCode::Internal))?;
                check_active(&prepare_control, deadline)?;
                if service.sync_identity(&account_id)?.revision != revision {
                    return Err(AppError::new(ErrorCode::Conflict));
                }
                // Secrets are resolved under the store lock, and sockets only
                // open after it is released. Do not resolve another secret after
                // an earlier blocking operation outlived cancellation/deadline.
                check_active(&prepare_control, deadline)?;
                let imap = service.sync_plan(&account_id);
                check_active(&prepare_control, deadline)?;
                let smtp = service.smtp_plan(&account_id);
                check_active(&prepare_control, deadline)?;
                Ok((imap, smtp))
            })
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))??;
            let imap = async {
                match plans.0 {
                    Err(error) => plan_failure(error),
                    Ok(plan) => {
                        // The receive guard bounds unsolicited responses as well
                        // as command timeouts; no LIST result is collected.
                        let mut session =
                            match imap::open_body_session(&plan.route, &plan.login).await {
                                Ok(session) => session,
                                Err(error) => return imap_failure(error),
                            };
                        let result = session.verify_noop().await;
                        let _ = session.logout().await;
                        match result {
                            Ok(()) => ProtocolVerification::verified(),
                            Err(error) => imap_failure(error),
                        }
                    }
                }
            };
            let smtp = async {
                match plans.1 {
                    Err(error) => plan_failure(error),
                    Ok(plan) => match smtp::verify_connection(
                        &plan.route,
                        &plan.login,
                        &smtp::Timeouts::default(),
                        &smtp::TlsSource::Platform,
                    )
                    .await
                    {
                        Ok(()) => ProtocolVerification::verified(),
                        Err(error) => smtp_failure(error),
                    },
                }
            };
            let (imap, smtp) = tokio::join!(imap, smtp);
            // A report only describes the saved version that was probed. This
            // metadata read never accesses the credential store.
            let service = self.service.clone();
            let account_id = request.account_id.clone();
            let final_control = cancelled.clone();
            tokio::task::spawn_blocking(move || {
                let service = service
                    .lock()
                    .map_err(|_| AppError::new(ErrorCode::Internal))?;
                check_active(&final_control, deadline)?;
                if service.sync_identity(&account_id)?.revision != revision {
                    return Err(AppError::new(ErrorCode::Conflict));
                }
                Ok(())
            })
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))??;
            Ok::<_, AppError>((imap, smtp))
        })
        .await;
        // Account updates and UI aborts may race a ready final metadata read.
        // Re-observe control before exposing either protocol's completed result.
        let final_control = *cancelled.borrow();
        if final_control != VerificationControl::Active {
            result = Err(final_control);
        } else if tokio::time::Instant::now() >= deadline {
            result = Err(VerificationControl::TimedOut);
        }
        let (imap, smtp) = match result {
            Ok(result) => result?,
            Err(VerificationControl::Invalidated) => {
                return Err(AppError::new(ErrorCode::Conflict));
            }
            Err(reason) => {
                let code = if reason == VerificationControl::TimedOut {
                    ConnectionVerificationCode::Timeout
                } else {
                    ConnectionVerificationCode::Cancelled
                };
                let failed =
                    ProtocolVerification::failed(ConnectionVerificationStage::Connection, code);
                (failed.clone(), failed)
            }
        };
        Ok(AccountConnectionVerification {
            account_id: request.account_id,
            revision,
            imap,
            smtp,
        })
    }

    pub fn cancel_account_connection_verification(
        &self,
        request: CancelConnectionVerification,
    ) -> Result<(), AppError> {
        validate_uuid(&request.account_id, "accountId")?;
        validate_uuid(&request.verification_id, "verificationId")?;
        self.verification
            .cancel(&request.account_id, &request.verification_id)
    }
}

fn check_active(
    control: &watch::Receiver<VerificationControl>,
    deadline: tokio::time::Instant,
) -> Result<(), AppError> {
    if *control.borrow() != VerificationControl::Active
        || tokio::time::Instant::now() >= deadline
        || control.has_changed().is_err()
    {
        return Err(AppError::new(ErrorCode::Conflict));
    }
    Ok(())
}

async fn bounded<T>(
    deadline: tokio::time::Instant,
    mut control: watch::Receiver<VerificationControl>,
    operation: impl std::future::Future<Output = T>,
) -> Result<T, VerificationControl> {
    tokio::select! {
        biased;
        reason = async {
            loop {
                let current = *control.borrow_and_update();
                if current != VerificationControl::Active {
                    return current;
                }
                if control.changed().await.is_err() {
                    return VerificationControl::Cancelled;
                }
            }
        } => Err(reason),
        _ = tokio::time::sleep_until(deadline) => Err(VerificationControl::TimedOut),
        result = operation => Ok(result),
    }
}

fn plan_failure(error: PlanError) -> ProtocolVerification {
    use ConnectionVerificationCode as C;
    let code = match error {
        PlanError::CredentialUntrusted => C::CredentialUntrusted,
        PlanError::CredentialMissing => C::CredentialMissing,
        PlanError::Account(error) => match error.code {
            ErrorCode::SecretStoreLocked | ErrorCode::SecretStoreUnavailable => C::SecretStore,
            _ => C::Configuration,
        },
    };
    ProtocolVerification::failed(ConnectionVerificationStage::Configuration, code)
}
fn imap_failure(error: ImapError) -> ProtocolVerification {
    use ConnectionVerificationCode as C;
    use ConnectionVerificationStage as S;
    let (stage, code) = match error {
        ImapError::Net(NetError::Proxy(_)) => (S::Connection, C::Proxy),
        ImapError::Net(NetError::Tls(_)) => (S::Tls, C::Tls),
        ImapError::Net(NetError::Connect(_)) => (S::Connection, C::Network),
        ImapError::Timeout | ImapError::Net(NetError::Timeout) => (S::Connection, C::Timeout),
        ImapError::Authentication(_) => (S::Authentication, C::Authentication),
        ImapError::ProviderPolicy => (S::Authentication, C::ProviderPolicy),
        ImapError::Unsupported(_) => (S::Connection, C::Unsupported),
        _ => (S::Mailbox, C::Protocol),
    };
    ProtocolVerification::failed(stage, code)
}
fn smtp_failure(outcome: SmtpOutcome) -> ProtocolVerification {
    use ConnectionVerificationCode as C;
    use ConnectionVerificationStage as S;
    let (stage, code) = match outcome {
        SmtpOutcome::Transient(failure) | SmtpOutcome::Permanent(failure) => match failure.reason {
            FailureReason::CredentialUntrusted => (S::Configuration, C::CredentialUntrusted),
            FailureReason::CredentialMissing => (S::Configuration, C::CredentialMissing),
            FailureReason::SecretStore => (S::Configuration, C::SecretStore),
            FailureReason::Proxy => (S::Connection, C::Proxy),
            FailureReason::TlsVerification | FailureReason::TlsUnavailable => (S::Tls, C::Tls),
            FailureReason::Timeout => (S::Connection, C::Timeout),
            FailureReason::Connect | FailureReason::Network => (S::Connection, C::Network),
            FailureReason::AuthRejected => (S::Authentication, C::Authentication),
            FailureReason::AuthUnavailable => (S::Authentication, C::AuthUnavailable),
            _ => (S::Connection, C::Protocol),
        },
        _ => (S::Connection, C::Protocol),
    };
    ProtocolVerification::failed(stage, code)
}

#[cfg(test)]
mod tests;
