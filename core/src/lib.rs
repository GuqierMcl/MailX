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
pub mod localization;
pub mod mail;
mod net;
pub mod secrets;
mod send;
pub mod settings;
mod smtp;
mod startup;
mod storage;
mod sync;

pub use accounts::AccountService;
pub use accounts::types::*;
pub use app::{
    AccountConnectionVerification, Activity, AttachmentFile, AttachmentInfo, AvatarImage,
    AvatarKind, BodyBinding, BodyDocument, BodyLinks, CancelConnectionVerification,
    CancelDraftVerification, ConnectionVerificationCode, ConnectionVerificationStage,
    ConnectionVerificationStatus, DraftConnectionVerification, FlagChange, MailxCore, MessageFlag,
    ProtocolVerification, RemoteImageStatus, VerifyAccountConnection, VerifyDraftConnection,
};
pub use drafts::attachments::{
    AddAttachmentRequest, AttachmentOrigin, AttachmentPolicy, DraftAttachmentSummary, InlineImage,
    RemoveAttachmentRequest,
};
pub use drafts::types::*;
pub use error::{ApiFailure, ApiResult, ApiSuccess, AppError, ErrorCode};
pub use events::{
    CoreEvent, EmailPreviewItem, EmailPreviews, EventSink, Events, NewMail, NewMailMessage,
};
pub use localization::{LanguagePreference, SupportedLocale};
pub use mail::*;
pub use net::images::ImageFetchError;
pub use send::types::*;
pub use settings::{
    AppSettings, AppearanceSettings, CloseBehavior, ComposeSettings, FontSize, GeneralSettings,
    LayoutSettings, MailTheme, NotificationSettings, NotificationSound, PreviewLines,
    RemoteImagePolicy, SecuritySettings, SettingsChanged, SettingsPatch, SettingsSnapshot,
    SettingsStore, ThemeMode, mail_body_css,
};
pub use startup::StartupStatus;
pub use sync::manager::{
    AccountSyncState, FlagWriteBack, FolderSyncPhase, FolderSyncState, SyncFailure,
    SyncFailureCode, SyncPhase,
};

/// The core crate version, available without initializing storage or networking.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
