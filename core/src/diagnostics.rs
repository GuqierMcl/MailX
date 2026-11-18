//! Host-supplied diagnostics containing only typed events and aggregate data.

use std::sync::Arc;

use serde::Serialize;

use crate::SyncFailureCode;

/// The host owns persistence. Callbacks run inline and should return quickly.
pub type DiagnosticSink = Arc<dyn Fn(DiagnosticEvent) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticBackend {
    Imap,
    Exchange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticContext {
    /// An opaque, process-local account identifier.
    pub account: u64,
    pub round: u64,
    pub backend: DiagnosticBackend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImapSearchKind {
    Since,
    Unseen,
    Uids,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImapFlagsPhase {
    Initial,
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImapDiagnosticOutcome {
    Complete,
    Incomplete,
    EmptyFromSelection,
    MissingEnumeration,
    InvalidEnumeration,
    UnrequestedUids,
    Rejected,
    Mailbox,
    Transport,
    Timeout,
    Protocol,
    Policy,
    Authentication,
    Unsupported,
    TooLarge,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DiagnosticEvent {
    Startup,
    StartupReady,
    StartupFailure {
        code: crate::ErrorCode,
    },
    SyncFailure {
        context: DiagnosticContext,
        code: SyncFailureCode,
        will_retry: bool,
    },
    SyncRetry {
        context: DiagnosticContext,
        delay_ms: u64,
    },
    SyncRecovery {
        context: DiagnosticContext,
    },
    ImapSearch {
        context: DiagnosticContext,
        kind: ImapSearchKind,
        outcome: ImapDiagnosticOutcome,
        requested: Option<usize>,
        matches: usize,
        selected_empty: Option<bool>,
        exists_changed: bool,
        uid_validity_changed: bool,
        esearch_advertised: bool,
        imap4rev2_advertised: bool,
    },
    ImapFlags {
        context: DiagnosticContext,
        phase: ImapFlagsPhase,
        outcome: ImapDiagnosticOutcome,
        requested: usize,
        returned: usize,
    },
}
