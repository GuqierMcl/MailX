// Generated from Rust by `cargo run -p mailx-core --example export_types`. Do not edit.

export type ErrorCode = "validation" | "notFound" | "conflict" | "idempotencyConflict" | "draftLocked" | "forbidden" | "alreadySending" | "bodyNotCached" | "unsupported" | "storageUnavailable" | "storageIncompatible" | "alreadyOpen" | "secretStoreLocked" | "secretStoreUnavailable" | "secretMissing" | "coreNotReady" | "internal";

export type AppError = { code: ErrorCode, message: string, field: string | null, };

export type StartupStatus = { "state": "initializing", revision: number, } | { "state": "ready", revision: number, } | { "state": "failed", revision: number, error: AppError, canRetry: boolean, };

export type ApiSuccess<T> = { ok: true, data: T, requestId: string, };

export type ApiFailure = { ok: false, error: AppError, requestId: string, };

export type ApiResult<T> = ApiSuccess<T> | ApiFailure;

export type TlsMode = "implicit" | "startTls";

export type ConnectionConfig = { host: string, port: number, tls: TlsMode, username: string, };

export type SecretUpdate = { "action": "keep" } | { "action": "replace", value: string, };

export type AuthenticationMode = "shared" | "separate";

export type PasswordAuthenticationInput = { "mode": "shared", password: SecretUpdate, } | { "mode": "separate", imapPassword: SecretUpdate, smtpPassword: SecretUpdate, };

export type OAuth2Configuration = { providerId: string, issuer: string, clientRegistrationId: string, authorizationSubject: string | null, targetMailboxResource: string | null, };

export type AuthenticationInput = { "kind": "password", } & ({ "mode": "shared", password: SecretUpdate, } | { "mode": "separate", imapPassword: SecretUpdate, smtpPassword: SecretUpdate, }) | { "kind": "oauth2", configuration: OAuth2Configuration, };

export type ProxyKind = "socks5" | "httpConnect";

export type DnsPolicy = "proxy";

export type ProxyAuthenticationInput = { username: string, password: SecretUpdate, };

export type RouteInput = { "kind": "direct" } | { "kind": "inherit" } | { "kind": "custom", proxyKind: ProxyKind, host: string, port: number, dns: DnsPolicy,
/**
 * HTTPS images may delegate target DNS to this configured proxy.
 */
imageProxyDns: boolean,
/**
 * Explicit grant for the endpoint supplied in this request. Never persisted.
 */
confirmImageProxyDns: boolean, authentication: ProxyAuthenticationInput | null, };

export type AccountProfileInput = { displayName: string,
/**
 * `#rrggbb`, or `null` for the neutral default.
 */
color: string | null,
/**
 * The name recipients see in `From`; `null` or empty sends with the bare
 * address. Unlike `displayName` it is never shown only to the user.
 */
senderName: string | null, };

export type ConnectionInput = { emailAddress: string, providerId: string | null, imap: ConnectionConfig, smtp: ConnectionConfig, authentication: AuthenticationInput, route: RouteInput, };

export type AccountConfigInput = { displayName: string,
/**
 * `#rrggbb`; omitted or `null` for the neutral default.
 */
color: string | null,
/**
 * The name recipients see in `From`; omitted, `null` or empty for none.
 */
senderName: string | null, emailAddress: string, providerId: string | null, imap: ConnectionConfig, smtp: ConnectionConfig, authentication: AuthenticationInput, route: RouteInput, };

export type CreateAccountRequest = { requestId: string, config: AccountConfigInput, };

export type UpdateConnectionRequest = { accountId: string, expectedRevision: number, connection: ConnectionInput, };

export type UpdateProfileRequest = { accountId: string, profile: AccountProfileInput, };

export type CredentialTrust = "trusted" | "needsConfirmation";

export type AuthenticationSummary = { id: string, generation: number, passwordConfigured: boolean, trust: CredentialTrust, };

export type PasswordAuthenticationSummary = { mode: AuthenticationMode, imap: AuthenticationSummary, smtp: AuthenticationSummary, };

export type ProxyAuthenticationSummary = { username: string, passwordConfigured: boolean, trust: CredentialTrust, };

export type RouteConfig = { "kind": "direct" } | { "kind": "inherit" } | { "kind": "custom", proxyKind: ProxyKind, host: string, port: number, dns: DnsPolicy,
/**
 * Defaults closed when reading snapshots written before this policy existed.
 */
imageProxyDns: boolean, authentication: ProxyAuthenticationSummary | null, };

export type ConnectionStatus = "unverified";

export type Account = { id: string,
/**
 * Version of the connection settings. Profile changes do not advance it.
 */
revision: number, displayName: string,
/**
 * `#rrggbb`, or `null` for the neutral default. Defaulted so credential
 * journals written before colors existed still load.
 */
color: string | null,
/**
 * The name recipients see in `From`, or `null` to send the bare address.
 */
senderName: string | null, emailAddress: string, providerId: string | null, mailboxSourceId: string, imap: ConnectionConfig, smtp: ConnectionConfig, authentication: PasswordAuthenticationSummary, route: RouteConfig, routeRevision: number,
/**
 * The initial application default is direct at revision 1.
 */
defaultRouteRevision: number | null, connectionStatus: ConnectionStatus, createdAt: string, updatedAt: string, };

export type VerifyAccountConnection = { accountId: string, expectedRevision: number,
/**
 * Unique per attempt, so closing an old view cannot cancel a newer attempt.
 */
verificationId: string, };

export type CancelConnectionVerification = { accountId: string, verificationId: string, };

export type ConnectionVerificationStatus = "verified" | "failed";

export type ConnectionVerificationStage = "configuration" | "connection" | "tls" | "authentication" | "mailbox" | "complete";

export type ConnectionVerificationCode = "credentialUntrusted" | "credentialMissing" | "secretStore" | "network" | "proxy" | "tls" | "timeout" | "authentication" | "authUnavailable" | "protocol" | "cancelled" | "unsupported" | "configuration";

export type ProtocolVerification = { status: ConnectionVerificationStatus, stage: ConnectionVerificationStage, code: ConnectionVerificationCode | null, };

export type AccountConnectionVerification = { accountId: string, revision: number, imap: ProtocolVerification, smtp: ProtocolVerification, };

export type FolderRole = "inbox" | "sent" | "drafts" | "archive" | "junk" | "trash" | "other";

export type Folder = { id: string, accountId: string, parentId: string | null, name: string, role: FolderRole,
/**
 * `false` for IMAP `\Noselect` containers that only group child folders.
 */
selectable: boolean,
/**
 * Whether a successful STATUS has supplied these counts. Unknown is not zero.
 */
countsKnown: boolean, unreadCount: number, totalCount: number, };

export type MailAddress = { name: string | null, address: string, };

export type BodyState = "missing" | "fetching" | "ready" | "failed";

export type BodyFailureCode = "tooLarge" | "stale" | "network" | "authentication" | "protocol" | "storage";

export type EmailSummary = { id: string, accountId: string, folderId: string, subject: string, from: MailAddress,
/**
 * Short plain-text excerpt for list rows, never HTML.
 */
preview: string,
/**
 * RFC 3339 timestamp.
 */
receivedAt: string, isRead: boolean, isStarred: boolean, hasAttachment: boolean, bodyState: BodyState,
/**
 * Local changes (read, star, move) not yet confirmed by the server.
 */
pendingChanges: boolean, };

export type EmailListQuery = {
/**
 * `None` searches every account.
 */
accountIds: Array<string> | null, folderId: string | null,
/**
 * Matches folders with this role in every selected account, e.g. a unified inbox.
 */
folderRole: FolderRole | null, text: string | null, unread: boolean | null, hasAttachment: boolean | null, receivedAfter: string | null, receivedBefore: string | null, limit: number | null, cursor: string | null, };

export type Coverage = { headersComplete: boolean,
/**
 * At least one matching selectable folder exceeded the per-round UID limit.
 */
windowCapped: boolean, bodiesComplete: boolean, syncedAfter: string | null, syncedBefore: string | null, };

export type EmailPage = { items: Array<EmailSummary>, nextCursor: string | null, coverage: Coverage, };

export type AttachmentSummary = { id: string, filename: string, mimeType: string, size: number, isInline: boolean, };

export type EmailBody = { text: string | null, hasHtml: boolean, truncated: boolean, remoteImagesBlocked: number, };

export type EmailDetail = { summary: EmailSummary, to: Array<MailAddress>, cc: Array<MailAddress>, replyTo: Array<MailAddress>,
/**
 * `None` until `summary.body_state` is `ready`.
 */
body: EmailBody | null, bodyFailure: BodyFailureCode | null, attachments: Array<AttachmentSummary>, };

export type SyncPhase = "syncing" | "idle" | "failed";

export type SyncFailureCode = "authentication" | "credentialUntrusted" | "credentialMissing" | "network" | "proxy" | "tls" | "timeout" | "unsupported" | "protocol" | "storage";

export type SyncFailure = { code: SyncFailureCode, message: string,
/**
 * Sanitized protocol or network reason, for display only.
 */
detail: string | null,
/**
 * Whether the worker will try again on its own.
 */
willRetry: boolean, };

export type FolderSyncPhase = "waiting" | "syncing" | "idle" | "failed";

export type FolderSyncState = { folderId: string, phase: FolderSyncPhase, lastSyncedAt: string | null, failure: SyncFailure | null, dataRevision: number, };

export type AccountSyncState = { accountId: string, mailboxSourceId: string | null,
/**
 * Incremented synchronously whenever account settings restart the worker.
 */
generation: number,
/**
 * Event order within the worker generation.
 */
sequence: number,
/**
 * Complete LIST commits; consumers refresh the folder tree on change.
 */
folderRevision: number, folders: Array<FolderSyncState>, phase: SyncPhase,
/**
 * RFC 3339 time of the last entirely successful round in this session.
 */
lastSyncedAt: string | null, failure: SyncFailure | null, };

export type ThemeMode = "system" | "light" | "dark";

export type FontSize = "small" | "default" | "large";

export type MailTheme = "follow" | "light";

export type AppearanceSettings = { theme: ThemeMode,
/**
 * `""` or a lowercase `#rrggbb` color.
 */
accentColor: string,
/**
 * `""` or comma-separated font families, tried in order.
 */
interfaceFont: string,
/**
 * `""` or comma-separated font families for message bodies, tried in order.
 */
mailFont: string, mailTheme: MailTheme, fontSize: FontSize, previewLines: PreviewLines, };

export type RemoteImagePolicy = "ask" | "automatic" | "never";

export type ImageFetchError = "targetBlocked" | "proxyPolicyRequired" | "dns" | "connect" | "proxy" | "tls" | "httpStatus" | "unsupportedType" | "invalidImage" | "tooLarge" | "timeout" | "cancelled";

export type RemoteImageStatus = { index: number, loading: boolean, error: ImageFetchError | null, };

export type SecuritySettings = { remoteImages: RemoteImagePolicy,
/**
 * Look sender avatars up on Gravatar, which learns a hash of each sender
 * address. Off by default.
 */
senderAvatars: boolean, };

export type LayoutSettings = {
/**
 * Width of the navigation sidebar, `220..=420`.
 */
navWidth: number,
/**
 * Width of the message list beside the reader, `300..=2000`, applied when
 * the window opens. In a window that is too narrow for it, list and reader
 * shrink together, each down to its own minimum.
 */
listWidth: number,
/**
 * The reader beside the message list is folded away. Unlike the sidebar,
 * this survives a restart: the window comes back as it was left.
 */
readerCollapsed: boolean, };

export type PreviewLines = "none" | "one" | "two" | "three";

export type NotificationSound = "system" | "chime" | "ding" | "droplet" | "bubble" | "marimba" | "sparkle" | "knock" | "chord" | "none";

export type NotificationSettings = { enabled: boolean,
/**
 * Include sender and subject. Off, a notification only says how many.
 */
showPreview: boolean,
/**
 * Also notify while the MailX window is in front.
 */
whenFocused: boolean, sound: NotificationSound,
/**
 * Accounts whose new mail never notifies.
 */
mutedAccounts: Array<string>, };

export type ComposeSettings = {
/**
 * Account used for new mail outside any account's folders; `""` for none.
 * It is not checked against the accounts here: a missing account falls
 * back to the first one when a draft is created.
 */
defaultAccountId: string,
/**
 * One of 0 (send at once), 5, 10, 20 or 30.
 */
undoSendSeconds: number, };

export type LanguagePreference = "system" | "zh-CN" | "en";

export type SupportedLocale = "zh-CN" | "en";

export type GeneralSettings = { language: LanguagePreference,
/**
 * Check for a new version shortly after start and every few hours. Off,
 * only the manual check in Settings → About runs.
 */
autoCheckUpdates: boolean, };

export type AppSettings = { general: GeneralSettings, appearance: AppearanceSettings, security: SecuritySettings, layout: LayoutSettings, notifications: NotificationSettings, compose: ComposeSettings, };

export type SettingsSnapshot = { settings: AppSettings,
/**
 * False until a settings file exists on disk.
 */
stored: boolean,
/**
 * The previous file could not be used and was moved to `settings.json.bak`.
 */
recovered: boolean, effectiveLocale: SupportedLocale,
/**
 * Monotonic within this settings-store lifetime; never persisted.
 */
revision: number, };

export type SettingsChanged = { revision: number, };

export type AvatarKind = "photo" | "icon";

export type FlagWriteBack = { accountId: string,
/**
 * Changes the server confirmed; the list's "waiting" markers may clear.
 */
sent: number,
/**
 * Starred changes kept on this device only because the folder cannot store
 * the flag.
 */
localOnlyStars: number, };

export type NewMailMessage = { emailId: string, from: MailAddress, subject: string,
/**
 * RFC 3339 timestamp.
 */
receivedAt: string, };

export type NewMail = { accountId: string, accountName: string,
/**
 * Every new message, including those not described below.
 */
count: number,
/**
 * The newest few, newest first. Empty when previews are turned off.
 */
messages: Array<NewMailMessage>, };

export type AvatarImage = { mimeType: string,
/**
 * Base64 of the re-encoded, validated image.
 */
data: string, kind: AvatarKind, };

export type DraftKind = "new" | "reply" | "replyAll" | "forward";

export type Draft = { id: string, accountId: string, kind: DraftKind,
/**
 * The message this draft answers or forwards. Cleared if that message
 * disappears; the quoted text and headers were copied at creation.
 */
sourceEmailId: string | null, to: Array<MailAddress>, cc: Array<MailAddress>, bcc: Array<MailAddress>, subject: string,
/**
 * Sanitized, authoritative editable HTML fragment.
 */
bodyHtml: string,
/**
 * Read-only plain-text projection generated by the core.
 */
bodyText: string,
/**
 * Original `Message-ID` in angle brackets, for replies.
 */
inReplyTo: string | null,
/**
 * Space-separated `<message-id>` list, for replies.
 */
references: string | null, createdAt: string, updatedAt: string,
/**
 * Advances on every saved change; updates must present the one they saw.
 */
revision: number, };

export type DraftSummary = { id: string, accountId: string, kind: DraftKind, sourceEmailId: string | null, to: Array<MailAddress>, subject: string,
/**
 * Start of the body on one line.
 */
preview: string, updatedAt: string, revision: number, };

export type CreateDraftRequest = {
/**
 * `null` picks the default sender account, else the first account.
 */
accountId: string | null, to: Array<MailAddress>, cc: Array<MailAddress>, bcc: Array<MailAddress>, subject: string, bodyHtml: string, clientMutationId: string, };

export type CreateDraftFromRequest = { emailId: string,
/**
 * `reply`, `replyAll` or `forward`.
 */
kind: DraftKind, clientMutationId: string, };

export type UpdateDraftRequest = { id: string, expectedRevision: number,
/**
 * `null` keeps the current sender account.
 */
accountId: string | null, to: Array<MailAddress>, cc: Array<MailAddress>, bcc: Array<MailAddress>, subject: string, bodyHtml: string, };

export type OutboxState = "queued" | "sending" | "accepted" | "failed" | "unknown" | "canceled";

export type OutboxResolution = "markedSent" | "resent";

export type UnknownResolution = "markSent" | "resend";

export type RejectedRecipientInfo = { address: string, code: number,
/**
 * Sanitized server text.
 */
text: string, };

export type OutboxError = {
/**
 * A fixed code such as `connect`, `authRejected`, `recipientsRejected`,
 * `rejectedAfterData`, `retryExhausted`, `accountChanged`, `unknown`.
 */
reason: string, smtpCode: number | null, text: string | null, rejected: Array<RejectedRecipientInfo>, };

export type ServerReplyInfo = { code: number, text: string, };

export type OutboxItem = { id: string,
/**
 * Cleared when the draft is deleted (after `accepted`, the item is the
 * local Sent copy and outlives its draft).
 */
draftId: string | null, accountId: string,
/**
 * With angle brackets.
 */
messageId: string, subject: string,
/**
 * The first few envelope recipients.
 */
recipients: Array<string>, recipientCount: number, state: OutboxState,
/**
 * Earliest time of the first attempt (end of the undo window).
 */
notBefore: string, dataStartedAt: string | null,
/**
 * Attempts started so far.
 */
attempts: number, nextAttemptAt: string | null, firstQueuedAt: string, error: OutboxError | null, acceptedAt: string | null, serverReply: ServerReplyInfo | null,
/**
 * The server's copy in a synced folder, once matched by Message-ID.
 */
serverCopyMessageId: string | null,
/**
 * Set on an `unknown` item the user has settled; it is then not live.
 */
resolution: OutboxResolution | null,
/**
 * The item queued by `resend` for a settled `unknown` one.
 */
supersededBy: string | null, createdAt: string, updatedAt: string, };

export type QueueSendRequest = { draftId: string, expectedRevision: number, clientMutationId: string, };

export type ResolveUnknownRequest = { id: string, resolution: UnknownResolution, clientMutationId: string, };

export type OutboxChanged = { accountId: string, outboxId: string, draftId: string | null, state: OutboxState, attempts: number, nextAttemptAt: string | null,
/**
 * The failure code when the item failed, was requeued after a failure or
 * is `unknown`.
 */
errorReason: string | null, };

export type DraftChange = "created" | "updated" | "locked" | "unlocked" | "deleted";

export type DraftChanged = { draftId: string, accountId: string, change: DraftChange, };
