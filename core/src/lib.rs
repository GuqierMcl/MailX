//! Shared, host-independent foundation for MailX.
//!
//! Hosts supply a data directory, system secret store and async runtime. The core owns
//! account configuration, IMAP sync and IDLE, cached message reading and search,
//! flag writes, deletion, local archiving, drafts, attachments and the SMTP send queue.
//! The transport-independent tool layer exposes authorized operations to local MCP;
//! AI provider connections are configured here; no mail goes to a model yet, and
//! the server host is not implemented.

pub mod accounts;
pub mod ai;
mod app;
pub mod assistant;
pub mod auth;
pub mod diagnostics;
pub mod drafts;
pub mod error;
pub mod events;
mod ews;
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
pub mod translate;

pub use accounts::AccountService;
pub use accounts::types::*;
pub use ai::{
    AiCatalog, AiCatalogProvider, AiConnection, AiModel, AiModelRef, AiModelSource, AiModelTarget,
    AiProtocol, AiProviderIcon, AiSettings, AiTestResult,
};
pub use app::{
    AccountConnectionVerification, AccountMailCount, Activity, AiConnectionState, AiService,
    AiState, ArchiveOutcome, AttachmentFile, AttachmentInfo, AvatarImage, AvatarKind,
    BeginOAuthAccount, BeginOAuthReauthorization, BodyBinding, BodyDocument, BodyLinks,
    CancelConnectionVerification, CancelDraftVerification, CodeCandidate, ConfirmOAuthMailbox,
    ConnectionVerificationCode, ConnectionVerificationStage, ConnectionVerificationStatus,
    CreateOAuthAccount, DayActivity, DraftBodyBinding, DraftConnectionVerification,
    EmailFlagResult, FlagBatchItem, FlagBatchItemOutcome, FlagBatchOutcome, FlagChange,
    MailOverview, MailOverviewQuery, MailxCore, MessageFlag, NewMailOverview,
    OAuthBrowserAuthorization, OAuthSessionHandle, OAuthSessionRequest, OAuthSignInResult,
    OverviewActivity, OverviewCode, OverviewContact, ProtocolVerification, RecentAttachment,
    RemoteImageStatus, SaveAiConnection, SavedAiConnection, TodayActivity, TranslationCacheStats,
    TranslationInfo, TranslationKeyState, TranslationProxyPasswordState, TranslationStatus,
    TranslationTest, VerificationCodeCleanup, VerificationCodeCopyOutcome, VerificationCodeState,
    VerificationCodeStatus, VerifyAccountConnection, VerifyDraftConnection,
};
pub use app::{DiscoverEwsSettings, EwsDiscoveryStatus, EwsSettingsDiscovery};
pub use assistant::{
    Assistant, AssistantEvent, AssistantEventKind, CachedEmailSummary, ChatContext,
    ComposeAssistState, ComposeTask, ConfirmAction, ConfirmAttachment, ConfirmDraftChange,
    ConfirmWarning, ConfirmationDetails, ConfirmationRequest, FinishedReason, FlagRestore,
    Occasion, OrganizeChange, PendingConfirmation, RewriteStyle, RunKind, RunKindTag, RunReasoning,
    RunSnapshot, RunState, StartRun, ToolActivity, ToolActivitySummary, ToolStatus, ToolUndo,
};
pub use assistant::{
    ChatTurn, ChatTurnConfirmation, ChatTurnReasoning, ChatTurnTool, ConfirmOutcome,
    ConversationChange, ConversationDetail, ConversationList, ConversationSummary,
    ConversationsChanged, TurnState,
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
    AiConsent, AppSettings, AppearanceSettings, AssistantMode, AssistantSendMode,
    AssistantSettings, BadgeSettings, CloseBehavior, ComposeOpenMode, ComposeSettings, FontSize,
    GeneralSettings, LayoutSettings, LayoutSettingsPatch, MailSettings, MailTheme, MailboxLabel,
    McpSettings, NotificationSettings, NotificationSound, NowArrangement, NowSettings,
    PreviewLines, ReadingMode, RemoteImagePolicy, SecuritySettings, ServiceRoute, SettingsChanged,
    SettingsPatch, SettingsSnapshot, SettingsStore, StartupView, ThemeMode, TranslationProviderId,
    TranslationSettings, WeekStartsOn, mail_body_css,
};
pub use startup::StartupStatus;
pub use sync::manager::{
    AccountSyncState, FlagWriteBack, FolderSyncPhase, FolderSyncState, MessagesDeleted,
    SyncFailure, SyncFailureCode, SyncPhase,
};

/// The core crate version, available without initializing storage or networking.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
