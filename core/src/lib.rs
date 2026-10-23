//! Shared, host-independent foundation for MailX.
//!
//! Account configuration and app preferences are persisted here; hosts supply a data directory,
//! system secret store and async runtime. Read-only background IMAP sync keeps
//! folders and per-folder message headers in a local cache for paginated queries.
//! Message bodies, flag writes and sending are not implemented yet.

pub mod accounts;
mod app;
pub mod drafts;
pub mod error;
pub mod events;
mod imap;
pub mod mail;
mod net;
pub mod secrets;
pub mod settings;
#[allow(dead_code, reason = "wired to the outbox and queue in sending step 3")]
mod smtp;
mod startup;
mod storage;
mod sync;

pub use accounts::AccountService;
pub use accounts::types::*;
pub use app::{
    AttachmentFile, AttachmentInfo, AvatarImage, AvatarKind, BodyBinding, BodyDocument, BodyLinks,
    FlagChange, MailxCore, MessageFlag,
};
pub use drafts::types::*;
pub use error::{ApiFailure, ApiResult, ApiSuccess, AppError, ErrorCode};
pub use events::{CoreEvent, EventSink, Events, NewMail, NewMailMessage};
pub use mail::*;
pub use settings::{
    AppSettings, AppearanceSettings, ComposeSettings, FontSize, LayoutSettings, MailTheme,
    NotificationSettings, NotificationSound, PreviewLines, RemoteImagePolicy, SecuritySettings,
    SettingsPatch, SettingsSnapshot, SettingsStore, ThemeMode, mail_body_css,
};
pub use startup::StartupStatus;
pub use sync::manager::{
    AccountSyncState, FlagWriteBack, FolderSyncPhase, FolderSyncState, SyncFailure,
    SyncFailureCode, SyncPhase,
};

/// The core crate version, available without initializing storage or networking.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
