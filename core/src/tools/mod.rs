//! The transport-agnostic tool layer shared by the local MCP adapter and the
//! built-in agent (ADR 0004, `docs/architecture/agent-tools.md`).
//!
//! Hosts build a [`RequestContext`] from what they know (who is connected and
//! what the user granted) and call [`Tools::call`]; tool input can never
//! widen that context. Calls block on storage like [`crate::MailxCore`], so
//! async hosts must run them in `spawn_blocking`. Nothing here depends on
//! Tauri, Axum, a UI or an MCP crate, and no tool can send mail.
mod accounts;
mod audit;
mod context;
mod drafts;
mod error;
mod mail;
mod spec;
#[cfg(test)]
mod tests;

pub use accounts::{AccountInfo, ListAccountsInput, ListAccountsOutput, SyncStatus};
pub use audit::{NoAudit, ToolAudit, ToolCallRecord};
pub use context::{AccountScope, Action, Audience, Grant, Principal, RequestContext};
pub use drafts::{CreateDraftInput, CreateDraftOutput};
pub use error::{ToolError, ToolErrorCode};
pub use mail::{
    Address, AttachmentInfo, EmailHit, ReadEmailInput, ReadEmailOutput, SearchEmailsInput,
    SearchEmailsOutput,
};
pub use spec::{Annotations, AudiencePolicy, ToolCall, ToolDescriptor, ToolSpec, Tools};
