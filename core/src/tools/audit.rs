//! Call records for hosts. The core has no logging backend, so the layer
//! hands each finished call to a host-provided sink; records never contain
//! mail content, tool arguments or results.
use super::error::ToolErrorCode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub request_id: String,
    /// `builtinAgent` or `localMcp`.
    pub principal_kind: &'static str,
    /// Run or connection id.
    pub principal_id: String,
    /// Self-reported MCP client name, display only.
    pub client_name: Option<String>,
    /// As requested, truncated; may name no registered tool.
    pub tool_id: String,
    /// Account and email ids the call touched.
    pub resource_ids: Vec<String>,
    /// `None` on success.
    pub error: Option<ToolErrorCode>,
}

pub trait ToolAudit: Send + Sync {
    fn record(&self, record: &ToolCallRecord);
}

/// Drops every record.
pub struct NoAudit;

impl ToolAudit for NoAudit {
    fn record(&self, _record: &ToolCallRecord) {}
}
