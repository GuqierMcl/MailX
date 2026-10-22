//! Shared, host-independent foundation for MailX.
//!
//! Account configuration and app preferences are persisted here; hosts supply a data directory,
//! system secret store and async runtime. Read-only background IMAP sync keeps
//! folders and per-folder message headers in a local cache for paginated queries.
//! Message bodies, flag writes and sending are not implemented yet.

pub mod accounts;
mod app;
pub mod error;
mod imap;
pub mod mail;
mod net;
pub mod secrets;
pub mod settings;
mod startup;
mod storage;
mod sync;

pub use accounts::AccountService;
pub use accounts::types::*;
pub use app::{
    AttachmentFile, AttachmentInfo, AvatarImage, AvatarKind, BodyBinding, BodyDocument, BodyLinks,
    MailxCore,
};
pub use error::{ApiFailure, ApiResult, ApiSuccess, AppError, ErrorCode};
pub use mail::*;
pub use settings::{
    AppSettings, AppearanceSettings, FontSize, LayoutSettings, MailTheme, RemoteImagePolicy,
    SecuritySettings, SettingsPatch, SettingsSnapshot, SettingsStore, ThemeMode, mail_body_css,
};
pub use startup::StartupStatus;
pub use sync::manager::{
    AccountSyncState, FolderSyncPhase, FolderSyncState, SyncFailure, SyncFailureCode, SyncListener,
    SyncPhase,
};

/// The core crate version, available without initializing storage or networking.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
