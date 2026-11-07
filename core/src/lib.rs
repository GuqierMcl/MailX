//! Shared, host-independent foundation for MailX.
//!
//! Hosts supply a data directory, system secret store and async runtime. The core owns
//! account configuration, IMAP sync and IDLE, cached message reading and search,
//! flag writes, deletion, local archiving, drafts, attachments and the SMTP send queue.
//! The transport-independent tool layer exposes authorized operations to local MCP;
//! built-in AI and the server host are not implemented yet.

pub mod accounts;
mod app;
pub mod auth;
pub mod drafts;
pub mod error;
pub mod events;
mod imap;
pub mod localization;
pub mod mail;
mod net;
mod providers;
pub mod secrets;
mod send;
pub mod settings;
mod smtp;
mod startup;
mod storage;
mod sync;
pub mod tools;

pub use accounts::AccountService;
pub use accounts::types::*;
pub use app::{
    AccountConnectionVerification, Activity, ArchiveOutcome, AttachmentFile, AttachmentInfo,
    AvatarImage, AvatarKind, BeginOAuthAccount, BeginOAuthReauthorization, BodyBinding,
    BodyDocument, BodyLinks, CancelConnectionVerification, CancelDraftVerification,
    ConnectionVerificationCode, ConnectionVerificationStage, ConnectionVerificationStatus,
    DraftBodyBinding, DraftConnectionVerification, EmailFlagResult, FlagChange, MailxCore,
    MessageFlag, OAuthBrowserAuthorization, OAuthSessionHandle, OAuthSessionRequest,
    ProtocolVerification, RemoteImageStatus, VerifyAccountConnection, VerifyDraftConnection,
};
pub use drafts::attachments::{
    AddAttachmentRequest, AttachmentOrigin, AttachmentPolicy, DraftAttachmentSummary, InlineImage,
    RemoveAttachmentRequest,
};
pub use drafts::types::*;
pub use error::{ApiFailure, ApiResult, ApiSuccess, AppError, ErrorCode};
pub use events::{
    CoreEvent, EmailPreviewItem, EmailPreviews, EventSink, Events, MailChangeSource,
    MailStateChangeKind, MailStateChanged, NewMail, NewMailMessage, SentCopiesChanged,
};
pub use localization::{LanguagePreference, SupportedLocale};
pub use mail::*;
pub use net::images::ImageFetchError;
pub use send::types::*;
pub use settings::{
    AppSettings, AppearanceSettings, CloseBehavior, ComposeSettings, FontSize, GeneralSettings,
    LayoutSettings, LayoutSettingsPatch, MailTheme, McpSettings, NotificationSettings,
    NotificationSound, PreviewLines, RemoteImagePolicy, SecuritySettings, SettingsChanged,
    SettingsPatch, SettingsSnapshot, SettingsStore, ThemeMode, mail_body_css,
};
pub use startup::StartupStatus;
pub use sync::manager::{
    AccountSyncState, FlagWriteBack, FolderSyncPhase, FolderSyncState, MessagesDeleted,
    SyncFailure, SyncFailureCode, SyncPhase,
};

/// The core crate version, available without initializing storage or networking.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
