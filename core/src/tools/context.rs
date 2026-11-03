//! Who is calling a tool and what they may do. Hosts build a
//! [`RequestContext`]; tool input can never widen it.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::Caller;

/// Which tool surface a principal sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    BuiltinAgent,
    Mcp,
}

/// The authenticated party behind a call, decided by the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Principal {
    /// MailX's own agent loop.
    BuiltinAgent { run_id: String },
    /// A client of the local MCP server. `client_name` is self-reported and
    /// only for display and audit, never for authorization.
    LocalMcp {
        connection_id: String,
        client_name: Option<String>,
    },
}

impl Principal {
    pub fn audience(&self) -> Audience {
        match self {
            Self::BuiltinAgent { .. } => Audience::BuiltinAgent,
            Self::LocalMcp { .. } => Audience::Mcp,
        }
    }

    /// Stable label for logs.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BuiltinAgent { .. } => "builtinAgent",
            Self::LocalMcp { .. } => "localMcp",
        }
    }

    /// Id of the run or connection, for logs.
    pub fn id(&self) -> &str {
        match self {
            Self::BuiltinAgent { run_id } => run_id,
            Self::LocalMcp { connection_id, .. } => connection_id,
        }
    }

    /// The send-path caller class. Both map to non-human callers, which the
    /// send checks reject: `LocalMcp` is [`Caller::Mcp`], `BuiltinAgent` is
    /// [`Caller::Model`].
    pub fn caller(&self) -> Caller {
        match self {
            Self::BuiltinAgent { .. } => Caller::Model,
            Self::LocalMcp { .. } => Caller::Mcp,
        }
    }
}

/// One capability a grant can hold. Strings match the MCP contract names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Action {
    #[serde(rename = "accounts:read")]
    AccountsRead,
    #[serde(rename = "mail:read")]
    MailRead,
    #[serde(rename = "mail:organize")]
    MailOrganize,
    #[serde(rename = "mail:archive")]
    MailArchive,
    #[serde(rename = "mail:sync")]
    MailSync,
    #[serde(rename = "draft:create")]
    DraftCreate,
    /// Update or delete existing drafts.
    #[serde(rename = "draft:write")]
    DraftWrite,
}

/// Accounts a grant covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountScope {
    All,
    Only(Vec<String>),
}

impl AccountScope {
    pub fn permits(&self, account_id: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(ids) => ids.iter().any(|id| id == account_id),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub accounts: AccountScope,
    pub actions: BTreeSet<Action>,
}

impl Grant {
    pub fn new(accounts: AccountScope, actions: impl IntoIterator<Item = Action>) -> Self {
        Self {
            accounts,
            actions: actions.into_iter().collect(),
        }
    }

    /// Local MCP grant: read, organize, archive and refresh mail, and manage
    /// drafts in every account. There is no send or delete-mail action.
    pub fn mcp_default() -> Self {
        Self::new(
            AccountScope::All,
            [
                Action::AccountsRead,
                Action::MailRead,
                Action::MailOrganize,
                Action::MailArchive,
                Action::MailSync,
                Action::DraftCreate,
                Action::DraftWrite,
            ],
        )
    }

    pub fn allows(&self, action: Action) -> bool {
        self.actions.contains(&action)
    }
}

/// Everything a host decides about one call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestContext {
    pub principal: Principal,
    pub grant: Grant,
    /// Correlates log lines; hosts pass their transport's request id.
    pub request_id: String,
}

impl RequestContext {
    pub fn new(principal: Principal, grant: Grant, request_id: impl Into<String>) -> Self {
        Self {
            principal,
            grant,
            request_id: request_id.into(),
        }
    }
}
