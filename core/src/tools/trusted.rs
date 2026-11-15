//! Data a tool call produces for the trusted host UI only. Tool output goes
//! to the model (or an MCP client); this never does.

/// A message's flags before a flag change, so the UI can restore them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviousFlags {
    pub email_id: String,
    pub is_read: bool,
    pub is_starred: bool,
}

/// Side results of one call, from [`Tools::call_traced`](super::Tools::call_traced).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustedOutput {
    /// One-use token of `undo_archive` for an archive or unarchive call that
    /// changed something.
    pub archive_undo_token: Option<String>,
    /// For `set_email_flags`: the flags of each checked message before the
    /// call (messages that were not changed are included).
    pub previous_flags: Vec<PreviousFlags>,
}
