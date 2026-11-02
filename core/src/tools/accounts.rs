use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::context::{Action, Audience};
use super::error::ToolError;
use super::spec::{Annotations, AudiencePolicy, ToolCall, ToolSpec, Tools};
use crate::{AccountSyncState, SyncFailureCode, SyncPhase};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListAccountsInput {}

/// How mail sync is doing for an account. Coarse by design: no server text,
/// credential detail or failure reason beyond the category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SyncStatus {
    /// The last sync round finished successfully.
    Ok,
    /// A sync round is running now.
    Syncing,
    /// The server rejected the login or the saved credential needs re-entry;
    /// the user must act in MailX.
    AuthError,
    /// Sync could not reach or talk to the server (network, proxy, TLS,
    /// timeout, protocol or local storage failure).
    ConnectionError,
    /// MailX has not completed or failed a sync round for this account yet
    /// (for example right after start). This does not mean a problem.
    Unknown,
}

impl SyncStatus {
    fn of(state: Option<&AccountSyncState>) -> Self {
        let Some(state) = state else {
            return Self::Unknown;
        };
        match state.phase {
            SyncPhase::Syncing => Self::Syncing,
            SyncPhase::Idle if state.last_synced_at.is_some() => Self::Ok,
            SyncPhase::Idle => Self::Unknown,
            SyncPhase::Failed => match state.failure.as_ref().map(|f| f.code) {
                Some(
                    SyncFailureCode::Authentication
                    | SyncFailureCode::CredentialMissing
                    | SyncFailureCode::CredentialUntrusted
                    | SyncFailureCode::ProviderPolicy,
                ) => Self::AuthError,
                _ => Self::ConnectionError,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    /// User-chosen label. Untrusted text for a model.
    pub display_name: String,
    pub email_address: String,
    pub sync_status: SyncStatus,
    /// RFC 3339 time of the last successful sync round since MailX started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListAccountsOutput {
    pub accounts: Vec<AccountInfo>,
}

pub(super) fn register(tools: &mut Tools) {
    let spec = ToolSpec::new::<ListAccountsInput, ListAccountsOutput>(
        "list_accounts",
        "List accounts",
        "List the mail accounts this connection may use: id, display name, email address and a coarse sync status \
         (ok, syncing, authError, connectionError or unknown; unknown only means no sync round has finished yet) and the time of the last successful sync. \
         Display names are user-provided text. Returns no credentials or server settings.",
        Annotations::READ_ONLY,
    )
    .requires(&[Action::AccountsRead])
    .visible_to(Audience::BuiltinAgent, AudiencePolicy::DEFAULT)
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(spec, list_accounts);
}

fn list_accounts(
    call: &ToolCall<'_>,
    _input: ListAccountsInput,
) -> Result<ListAccountsOutput, ToolError> {
    let states = call.core.sync_states();
    let accounts = call
        .core
        .list_accounts()?
        .into_iter()
        .filter(|account| call.ctx.grant.accounts.permits(&account.id))
        .map(|account| {
            call.note_resource(&account.id);
            let state = states.iter().find(|s| s.account_id == account.id);
            AccountInfo {
                sync_status: SyncStatus::of(state),
                last_synced_at: state.and_then(|s| s.last_synced_at.clone()),
                id: account.id,
                display_name: account.display_name,
                email_address: account.email_address,
            }
        })
        .collect();
    Ok(ListAccountsOutput { accounts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SyncFailure;

    fn state(phase: SyncPhase, synced: bool, code: Option<SyncFailureCode>) -> AccountSyncState {
        AccountSyncState {
            account_id: "a".into(),
            mailbox_source_id: None,
            generation: 0,
            sequence: 0,
            folder_revision: 0,
            folders: Vec::new(),
            phase,
            last_synced_at: synced.then(|| "2026-01-01T00:00:00Z".to_owned()),
            failure: code.map(|code| SyncFailure {
                code,
                message: "server said secret".into(),
                detail: Some("imap.example.com".into()),
                will_retry: true,
            }),
        }
    }

    #[test]
    fn sync_status_follows_phase_and_failure_code() {
        use SyncFailureCode as C;
        use SyncStatus as S;
        assert_eq!(S::of(None), S::Unknown);
        assert_eq!(S::of(Some(&state(SyncPhase::Idle, true, None))), S::Ok);
        assert_eq!(
            S::of(Some(&state(SyncPhase::Idle, false, None))),
            S::Unknown
        );
        assert_eq!(
            S::of(Some(&state(SyncPhase::Syncing, true, None))),
            S::Syncing
        );
        for code in [
            C::Authentication,
            C::CredentialMissing,
            C::CredentialUntrusted,
            C::ProviderPolicy,
        ] {
            assert_eq!(
                S::of(Some(&state(SyncPhase::Failed, false, Some(code)))),
                S::AuthError
            );
        }
        for code in [
            C::Network,
            C::Proxy,
            C::Tls,
            C::Timeout,
            C::Unsupported,
            C::Protocol,
            C::Storage,
        ] {
            assert_eq!(
                S::of(Some(&state(SyncPhase::Failed, true, Some(code)))),
                S::ConnectionError
            );
        }
        assert_eq!(
            S::of(Some(&state(SyncPhase::Failed, false, None))),
            S::ConnectionError
        );
    }
}
