//! Shared, host-independent foundation for MailX.
//!
//! Account configuration is persisted here; hosts supply a data directory and
//! system secret store. No network login, synchronization, or sending is enabled.

pub mod accounts;
pub mod error;
pub mod mail;
pub mod secrets;
mod startup;
mod storage;

pub use accounts::AccountService;
pub use accounts::types::*;
pub use error::{ApiFailure, ApiResult, ApiSuccess, AppError, ErrorCode};
pub use mail::*;
pub use startup::StartupStatus;

/// The core crate version, available without initializing storage or networking.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
