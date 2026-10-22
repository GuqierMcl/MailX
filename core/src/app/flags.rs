//! The flag names and results of a user's read / starred change, as hosts see
//! them. The storage layer has its own copy of the flag type with IMAP details.
use crate::storage::Flag;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageFlag {
    Read,
    Starred,
}

impl From<MessageFlag> for Flag {
    fn from(flag: MessageFlag) -> Self {
        match flag {
            MessageFlag::Read => Flag::Seen,
            MessageFlag::Starred => Flag::Flagged,
        }
    }
}

/// What the local change turned into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagChange {
    /// The message already had that value.
    Unchanged,
    /// Saved here; the server will be updated in the background.
    Queued,
    /// Saved only on this device: the server cannot keep this flag in that
    /// folder. Hosts tell the user, once.
    LocalOnly,
}
