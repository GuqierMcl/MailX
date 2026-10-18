//! Shared, host-independent foundation for MailX.
//!
//! Account configuration is persisted here; hosts supply a data directory,
//! system secret store and async runtime. Background sync logs in over IMAP to
//! keep each account's folder list current; messages are not synced and nothing
//! is sent yet.

pub mod accounts;
mod app;
pub mod error;
mod imap;
pub mod mail;
mod net;
pub mod secrets;
mod startup;
mod storage;
mod sync;

pub use accounts::AccountService;
pub use accounts::types::*;
pub use app::MailxCore;
pub use error::{ApiFailure, ApiResult, ApiSuccess, AppError, ErrorCode};
pub use mail::*;
pub use startup::StartupStatus;
pub use sync::manager::{AccountSyncState, SyncFailure, SyncFailureCode, SyncListener, SyncPhase};

/// The core crate version, available without initializing storage or networking.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
