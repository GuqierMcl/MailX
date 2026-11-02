use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::context::{Action, Audience};
use super::error::ToolError;
use super::spec::{Annotations, AudiencePolicy, ToolCall, ToolSpec, Tools};
use crate::ConnectionStatus;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListAccountsInput {}

/// Coarse account state; no server or credential detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AccountStatus {
    /// Saved but never confirmed to connect.
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    /// User-chosen label. Untrusted text for a model.
    pub display_name: String,
    pub email_address: String,
    pub status: AccountStatus,
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
        "List the mail accounts this connection may use: id, display name, email address and a coarse status. \
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
    let accounts = call
        .core
        .list_accounts()?
        .into_iter()
        .filter(|account| call.ctx.grant.accounts.permits(&account.id))
        .map(|account| {
            call.note_resource(&account.id);
            AccountInfo {
                status: match account.connection_status {
                    ConnectionStatus::Unverified => AccountStatus::Unverified,
                },
                id: account.id,
                display_name: account.display_name,
                email_address: account.email_address,
            }
        })
        .collect();
    Ok(ListAccountsOutput { accounts })
}
