// Generated from Rust by `cargo run -p mailx-core --example export_types`. Do not edit.

export type ErrorCode = "validation" | "notFound" | "conflict" | "idempotencyConflict" | "draftLocked" | "forbidden" | "alreadySending" | "confirmationRequired" | "alreadyDeleting" | "bodyNotCached" | "attachmentTooLarge" | "tooManyAttachments" | "unsupportedImage" | "unsupported" | "oauthNetwork" | "oauthProxy" | "oauthTls" | "oauthRateLimited" | "oauthServerUnavailable" | "oauthTimeout" | "oauthAuthorizationDenied" | "oauthSessionExpired" | "oauthInvalidResponse" | "oauthReauthorizationRequired" | "oauthMailboxRefused" | "oauthUsePersonalEntry" | "oauthUseOrganizationEntry" | "translationNotConfigured" | "translationConsentRequired" | "translationKeyRejected" | "translationQuotaExceeded" | "translationRateLimited" | "translationNetwork" | "translationProxy" | "translationTimeout" | "translationUnavailable" | "translationInvalidResponse" | "translationLanguageUnsupported" | "translationTooLarge" | "translationCancelled" | "aiNotConfigured" | "aiKeyRejected" | "aiQuotaExceeded" | "aiModelNotFound" | "aiRateLimited" | "aiRequestRejected" | "aiUnavailable" | "aiNetwork" | "aiProxy" | "aiTimeout" | "aiInvalidResponse" | "assistantDisabled" | "assistantBusy" | "assistantContextFull" | "storageUnavailable" | "storageIncompatible" | "alreadyOpen" | "secretStoreLocked" | "secretStoreUnavailable" | "secretMissing" | "coreNotReady" | "internal";

export type AppError = { code: ErrorCode, message: string, field: string | null, };

export type StartupStatus = { "state": "initializing", revision: number, } | { "state": "ready", revision: number, } | { "state": "failed", revision: number, error: AppError, canRetry: boolean, };

export type ApiSuccess<T> = { ok: true, data: T, requestId: string, };

export type ApiFailure = { ok: false, error: AppError, requestId: string, };

export type ApiResult<T> = ApiSuccess<T> | ApiFailure;

export type TlsMode = "implicit" | "startTls";

export type ConnectionConfig = { host: string, port: number, tls: TlsMode, username: string, };

export type EwsConnectionConfig = { url: string, username: string, };

export type EwsBackend = "ews";

export type MailConnection = { backend: EwsBackend, ews: EwsConnectionConfig, } | { imap: ConnectionConfig, smtp: ConnectionConfig, };

export type SecretUpdate = { "action": "keep" } | { "action": "replace", value: string, };

export type AuthenticationMode = "shared" | "separate";

export type PasswordAuthenticationInput = { "mode": "shared", password: SecretUpdate, } | { "mode": "separate", imapPassword: SecretUpdate, smtpPassword: SecretUpdate, };

export type OAuth2Configuration = { providerId: string, issuer: string, clientRegistrationId: string, authorizationSubject: string | null, targetMailboxResource: string | null, };

export type BeginOAuthAccount = { providerId: string,
/**
 * How MailX reaches the provider; also the new account's route.
 */
route: RouteInput, };

export type BeginOAuthReauthorization = { providerId: string, accountId: string, expectedRevision: number, };

export type OAuthSessionRequest = { sessionId: string, };

export type OAuthSessionHandle = { sessionId: string, };

export type ConfirmOAuthMailbox = { sessionId: string, emailAddress: string, };

export type CreateOAuthAccount = { sessionId: string, requestId: string, profile: AccountProfileInput, };

export type OAuthSignInResult = { "kind": "verified", emailAddress: string, } | { "kind": "needsMailbox", suggestion: string | null, };

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
senderName: string | null,
/**
 * Prefer known provider artwork when available; omitted defaults to true.
 */
useProviderIcon: boolean, };

export type ConnectionInput = { emailAddress: string, providerId: string | null, authentication: AuthenticationInput, route: RouteInput, } & ({ backend: EwsBackend, ews: EwsConnectionConfig, } | { imap: ConnectionConfig, smtp: ConnectionConfig, });

export type AccountConfigInput = { displayName: string,
/**
 * `#rrggbb`; omitted or `null` for the neutral default.
 */
color: string | null,
/**
 * The name recipients see in `From`; omitted, `null` or empty for none.
 */
senderName: string | null,
/**
 * Prefer known provider artwork when available; omitted defaults to true.
 */
useProviderIcon: boolean, emailAddress: string, providerId: string | null, authentication: AuthenticationInput, route: RouteInput, } & ({ backend: EwsBackend, ews: EwsConnectionConfig, } | { imap: ConnectionConfig, smtp: ConnectionConfig, });

export type CreateAccountRequest = { requestId: string, config: AccountConfigInput, };

export type UpdateConnectionRequest = { accountId: string, expectedRevision: number, connection: ConnectionInput, };

export type UpdateProfileRequest = { accountId: string, profile: AccountProfileInput, };

export type RemoveAccountRequest = { accountId: string, expectedRevision: number, };

export type AccountRemoval = { accountId: string,
/**
 * Includes every deleted local draft so hosts can retire its editor.
 */
draftIds: Array<string>,
/**
 * Local removal committed; host-owned credentials will be retried later.
 */
credentialsPending: boolean, };

export type CredentialTrust = "trusted" | "needsConfirmation";

export type AuthenticationSummary = { id: string, generation: number, passwordConfigured: boolean, trust: CredentialTrust, };

export type PasswordAuthenticationSummary = { mode: AuthenticationMode, imap: AuthenticationSummary, smtp: AuthenticationSummary, };

export type EwsAuthenticationSummary = { credential: AuthenticationSummary, };

export type ProtocolAuthenticationSummary = { id: string, generation: number, credentialConfigured: boolean, trust: CredentialTrust, };

export type OAuthAuthenticationState = "connected" | "needsReauthorization";

export type OAuthAuthenticationSummary = { configuration: OAuth2Configuration, state: OAuthAuthenticationState, imap: ProtocolAuthenticationSummary, smtp: ProtocolAuthenticationSummary, };

export type AccountAuthenticationSummary = { "kind": "password" } & PasswordAuthenticationSummary | { "kind": "oauth2" } & OAuthAuthenticationSummary | { "kind": "ewsNtlm" } & EwsAuthenticationSummary;

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
senderName: string | null,
/**
 * Appearance only. Defaults to true for credential journals from older builds.
 */
useProviderIcon: boolean, emailAddress: string, providerId: string | null, mailboxSourceId: string, authentication: AccountAuthenticationSummary, route: RouteConfig, routeRevision: number,
/**
 * The initial application default is direct at revision 1.
 */
defaultRouteRevision: number | null, connectionStatus: ConnectionStatus, createdAt: string, updatedAt: string, } & ({ backend: EwsBackend, ews: EwsConnectionConfig, } | { imap: ConnectionConfig, smtp: ConnectionConfig, });

export type DiscoverEwsSettings = { emailAddress: string, route: RouteInput, targetGrant: string | null, username: string | null, password: string | null, };

export type EwsDiscoveryStatus = "found" | "authRequired" | "confirmationRequired" | "notFound" | "rejected" | "network" | "proxy" | "tls" | "timeout" | "unsupported" | "protocol";

export type EwsSettingsDiscovery = { status: EwsDiscoveryStatus, endpoint: EwsConnectionConfig | null, source: string | null, targetGrant: string | null, };

export type VerifyAccountConnection = { accountId: string, expectedRevision: number,
/**
 * Unique per attempt, so closing an old view cannot cancel a newer attempt.
 */
verificationId: string, };

export type CancelConnectionVerification = { accountId: string, verificationId: string, };

export type VerifyDraftConnection = { verificationId: string, connection: ConnectionInput, };

export type CancelDraftVerification = { verificationId: string, };

export type ConnectionVerificationStatus = "verified" | "failed";

export type ConnectionVerificationStage = "configuration" | "connection" | "tls" | "authentication" | "mailbox" | "complete";

export type ConnectionVerificationCode = "credentialUntrusted" | "credentialMissing" | "secretStore" | "network" | "proxy" | "tls" | "timeout" | "authentication" | "providerPolicy" | "authUnavailable" | "protocol" | "cancelled" | "unsupported" | "configuration";

export type ProtocolVerification = { status: ConnectionVerificationStatus, stage: ConnectionVerificationStage, code: ConnectionVerificationCode | null, };

export type AccountConnectionVerification = { accountId: string, revision: number, imap: ProtocolVerification, smtp: ProtocolVerification, ews?: ProtocolVerification, };

export type DraftConnectionVerification = { verificationId: string, imap: ProtocolVerification, smtp: ProtocolVerification, ews?: ProtocolVerification, };

export type FolderRole = "inbox" | "sent" | "drafts" | "archive" | "junk" | "trash" | "other";

export type Folder = { id: string, accountId: string, parentId: string | null, name: string, role: FolderRole,
/**
 * `false` for IMAP `\Noselect` containers that only group child folders.
 */
selectable: boolean,
/**
 * Whether a successful STATUS has supplied these counts. Unknown is not zero.
 */
countsKnown: boolean,
/**
 * Server unread count; for an inbox minus the locally archived unread mail.
 */
unreadCount: number, totalCount: number,
/**
 * Locally archived messages of this inbox (0 for other folders).
 */
archivedCount: number,
/**
 * Locally archived unread messages of this inbox (0 for other folders).
 */
archivedUnreadCount: number, };

export type MailAddress = { name: string | null, address: string, };

export type EmailRecipients = {
/**
 * At most three addresses.
 */
addresses: Array<MailAddress>, total: number, };

export type BodyState = "missing" | "fetching" | "ready" | "failed";

export type BodyFailureCode = "tooLarge" | "stale" | "network" | "authentication" | "providerPolicy" | "protocol" | "storage";

export type PreviewState = "missing" | "ready" | "failed";

export type EmailSummary = { id: string, accountId: string, folderId: string, subject: string, from: MailAddress,
/**
 * Leading recipients for Sent-style rows (at most three) and the total.
 */
to: EmailRecipients,
/**
 * Short plain-text excerpt for list rows, never HTML.
 */
preview: string,
/**
 * RFC 3339 timestamp.
 */
receivedAt: string, isRead: boolean, isStarred: boolean, hasAttachment: boolean, bodyState: BodyState, previewState: PreviewState,
/**
 * Local changes (read, star, move) not yet confirmed by the server.
 */
pendingChanges: boolean,
/**
 * A copy of an accepted send kept only on this device (`local-sent:` ids).
 */
localOnly: boolean,
/**
 * Archived on this device only (inbox messages; the server still has it in INBOX).
 */
archived: boolean,
/**
 * Plain-text excerpt around a full-text hit in the body or attachment
 * names; `None` when there is no text search or the hit is only in the
 * subject or people.
 */
matchSnippet: Array<SnippetSegment> | null,
/**
 * Set when this row stands for every message of an aggregated sender in
 * the list (see [`EmailListQuery::group_senders`]); the row itself is
 * that sender's newest message in scope.
 */
senderGroup: SenderGroup | null, };

export type SenderGroup = {
/**
 * Normalized (ASCII lowercase) sender address.
 */
address: string, total: number, unread: number, };

export type ContactAggregation = {
/**
 * Normalized (ASCII lowercase) sender address.
 */
address: string,
/**
 * Display name of the newest visible message from this address.
 */
name: string | null,
/**
 * Visible messages from this address in every account, outside junk and trash.
 */
messageCount: number, latestReceivedAt: string | null, createdAt: string, };

export type ContactSuggestion = {
/**
 * The newest non-empty display name seen with the address, trimmed.
 * Untrusted text.
 */
name: string | null,
/**
 * ASCII-lowercased.
 */
address: string, };

export type SnippetSegment = { text: string, hit: boolean, };

export type EmailListQuery = {
/**
 * `None` searches every account.
 */
accountIds: Array<string> | null, folderId: string | null,
/**
 * Matches folders with this role in every selected account, e.g. a unified inbox.
 */
folderRole: FolderRole | null,
/**
 * Folders with these roles are left out, e.g. junk and trash in the
 * Starred view. Only narrows the result; an explicit `folder_role` is
 * not affected by it. Empty (the default) excludes nothing.
 */
excludeFolderRoles: Array<FolderRole>,
/**
 * `None` hides archived messages, except in a text search, which finds them.
 */
archive: ArchiveScope | null, text: string | null, unread: boolean | null,
/**
 * Messages that pass the `unread` filter whatever their read state, so
 * mail opened from an Unread view stays listed after it is read. Every
 * other filter still applies. At most [`MAX_KEEP_IDS`].
 */
keepIds: Array<string>,
/**
 * Match the local desired star state, including pending server writes.
 */
starred: boolean | null, hasAttachment: boolean | null,
/**
 * Only messages from this sender address (case-insensitive). Local Sent
 * copies match on the account's own address.
 */
fromAddress: string | null,
/**
 * Collapse messages of aggregated senders into one row each. Ignored in
 * a text search and together with `from_address`.
 */
groupSenders: boolean, receivedAfter: string | null, receivedBefore: string | null, limit: number | null, cursor: string | null, };

export type Coverage = { headersComplete: boolean,
/**
 * At least one matching selectable folder exceeded the per-round UID limit.
 */
windowCapped: boolean, bodiesComplete: boolean,
/**
 * Messages in scope whose body text is in the local search index.
 */
indexedBodies: number, syncedAfter: string | null, syncedBefore: string | null, };

export type EmailPage = { items: Array<EmailSummary>, nextCursor: string | null, coverage: Coverage, };

export type MailOverviewQuery = {
/**
 * RFC 3339; the last time the user looked at the Now page. `None` asks
 * for no new-mail section: `newMail` is then empty. The caller decides
 * what to send on first use (for example the start of today).
 */
since: string | null,
/**
 * RFC 3339; exactly fourteen strictly ascending instants, each the start
 * of a local day, the last one the start of today and not in the future.
 * Day `i` runs to the next start, the last day to now. Omitted or `null`
 * asks for no activity section.
 */
dayStarts?: Array<string>, };

export type AccountMailCount = { accountId: string, count: number, };

export type NewMailOverview = { total: number,
/**
 * Only accounts with new mail, ordered by account id.
 */
accounts: Array<AccountMailCount>,
/**
 * The newest five, newest first.
 */
latest: Array<EmailSummary>, };

export type CodeCandidate = { emailId: string, accountId: string, folderId: string, from: MailAddress,
/**
 * RFC 3339.
 */
receivedAt: string,
/**
 * The stored body state, as in [`OverviewCode`].
 */
bodyState: BodyState, };

export type OverviewCode = { emailId: string, accountId: string, folderId: string, from: MailAddress, code: string,
/**
 * RFC 3339.
 */
receivedAt: string,
/**
 * The stored body state, so hosts can key the shared verification-code
 * status (which reads the full body) to the same message version.
 */
bodyState: BodyState, };

export type RecentAttachment = { email: EmailSummary,
/**
 * Names of the non-inline attachments, known once the body has been
 * downloaded; empty before that. Untrusted text from the sender.
 */
fileNames: Array<string>, };

export type DayActivity = {
/**
 * RFC 3339 UTC; the requested day start, normalized.
 */
start: string, received: number, };

export type OverviewContact = {
/**
 * Lowercased.
 */
address: string,
/**
 * The most recent non-empty display name among the counted messages.
 * Untrusted text.
 */
name: string | null, count: number, };

export type TodayActivity = {
/**
 * Inbox mail received since the last day start, as the inbox lists it:
 * locally archived mail is left out.
 */
received: number,
/**
 * How many of those are unread.
 */
unread: number, };

export type OverviewActivity = {
/**
 * One entry per requested day start, oldest first: inbox mail received
 * that day, locally archived mail included. Junk, Trash and Sent never
 * count. The last day ends at `generatedAt`.
 */
days: Array<DayActivity>,
/**
 * Sum of the last seven days.
 */
receivedLastWeek: number,
/**
 * Sum of the seven days before them.
 */
receivedPreviousWeek: number,
/**
 * Messages of the unified Sent view (sent-role folders and local Sent
 * copies, a copy matched to its server copy counted once) in the last
 * seven days.
 */
sentLastWeek: number,
/**
 * The same for the seven days before them.
 */
sentPreviousWeek: number,
/**
 * Whether the local cache can be trusted to hold both weeks, so the
 * week-over-week comparison means something. True only when every
 * account has an inbox and every current inbox and sent-role folder
 * generation has finished at least one sync round not capped by the
 * per-round limit (a later round in progress keeps it), is not capped
 * now, and started its sync window at least one UTC day before the
 * first day start (the margin covers servers that read the window's
 * date in their own time zone).
 */
previousWeekComplete: boolean,
/**
 * At most three addresses with the most messages in the last seven days:
 * inbox mail received from them (archived included) plus Sent view
 * messages addressed to them. Only addresses the Sent view holds at least
 * one message to qualify, and the accounts' own addresses never do.
 * Most messages first, then the most recent.
 */
topContacts: Array<OverviewContact>, today: TodayActivity, };

export type MailOverview = { newMail: NewMailOverview,
/**
 * Starred unread messages, in the Starred view's scope.
 */
starredUnread: number,
/**
 * Newest first, at most three, from the last fifteen minutes.
 */
verificationCodes: Array<OverviewCode>,
/**
 * Inbox messages from the same fifteen minutes whose subject looks like
 * a verification mail but that have no code here yet (no preview, no
 * downloaded body, or none found). Newest first, at most five. Hosts
 * hand them to the shared verification-code recognizer, which may
 * download the body; one counts as a code only once it answers.
 */
codeCandidates: Array<CodeCandidate>,
/**
 * Newest first, at most five inbox messages from the last seven days.
 */
recentAttachments: Array<RecentAttachment>,
/**
 * Received, sent and contact statistics of the last two weeks; present
 * only when the query sent `dayStarts`.
 */
activity?: OverviewActivity,
/**
 * RFC 3339; the core's clock when the overview was computed. The code
 * and attachment windows end here, and so does the last activity day.
 */
generatedAt: string, };

export type ConversationPage = { messages: Array<EmailSummary>,
/**
 * True when local headers might be incomplete (some older messages may be missing).
 */
possiblyIncomplete: boolean, };

export type SelectedEmail = { id: string, accountId: string, folderId: string, isRead: boolean, isStarred: boolean, archived: boolean, localOnly: boolean, };

export type EmailSelection = { items: Array<SelectedEmail>, truncated: boolean, };

export type AttachmentSummary = { id: string, filename: string, mimeType: string, size: number, isInline: boolean, };

export type EmailBody = { text: string | null, hasHtml: boolean, truncated: boolean, remoteImagesBlocked: number, };

export type EmailDetail = { summary: EmailSummary, to: Array<MailAddress>, cc: Array<MailAddress>, replyTo: Array<MailAddress>,
/**
 * `None` until `summary.body_state` is `ready`.
 */
body: EmailBody | null, bodyFailure: BodyFailureCode | null, attachments: Array<AttachmentSummary>, };

export type VerificationCodeState = "unchecked" | "ready" | "notFound" | "ambiguous" | "unavailable";

export type VerificationCodeStatus = { emailId: string, state: VerificationCodeState, code: string | null, recognitionId: string | null, };

export type VerificationCodeCleanup = "disabled" | "queued" | "unavailable" | "failed";

export type VerificationCodeCopyOutcome = { emailId: string, accountId: string, cleanup: VerificationCodeCleanup, deletion: EmailDeletion | null, };

export type SyncPhase = "syncing" | "idle" | "failed";

export type SyncFailureCode = "authentication" | "signInRequired" | "providerPolicy" | "credentialUntrusted" | "credentialMissing" | "network" | "proxy" | "tls" | "timeout" | "unsupported" | "protocol" | "flagsIncomplete" | "storage";

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

export type FolderSyncState = { folderId: string, phase: FolderSyncPhase, lastSyncedAt: string | null, failure: SyncFailure | null,
/**
 * RFC 3339 time the current run of failures began; cleared on success.
 * Survives round boundaries so a host can wait before showing a
 * retryable failure that may just be a network blip.
 */
failureSince: string | null, dataRevision: number, };

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
lastSyncedAt: string | null, failure: SyncFailure | null,
/**
 * RFC 3339 time the current run of retryable failures began; cleared by a
 * fully successful round or a new worker generation.
 */
failureSince: string | null, };

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
mailFont: string, mailTheme: MailTheme, fontSize: FontSize, previewLines: PreviewLines, mailboxLabel: MailboxLabel, };

export type RemoteImagePolicy = "ask" | "automatic" | "never";

export type ImageFetchError = "targetBlocked" | "proxyPolicyRequired" | "dns" | "connect" | "proxy" | "tls" | "httpStatus" | "notFound" | "unsupportedType" | "invalidImage" | "tooLarge" | "timeout" | "cancelled";

export type RemoteImageRole = "decorative" | "content" | "important";

export type RemoteImageStatus = { index: number,
/**
 * First occurrence of this URL in the bound document.
 */
group: number, role: RemoteImageRole, loading: boolean, error: ImageFetchError | null, };

export type SecuritySettings = { remoteImages: RemoteImagePolicy,
/**
 * Look sender avatars up on Gravatar, which learns a hash of each sender
 * address. Off by default.
 */
senderAvatars: boolean, };

export type ReadingMode = "split" | "single";

export type LayoutSettings = {
/**
 * Width of the navigation sidebar, `220..=420`.
 */
navWidth: number,
/**
 * The navigation sidebar is folded to its icon rail. A narrow window may
 * show the rail on its own without changing this.
 */
navCollapsed: boolean,
/**
 * Width of the message list beside the reader, `300..=2000`, applied when
 * the window opens. In a window that is too narrow for it, list and reader
 * shrink together, each down to its own minimum.
 */
listWidth: number,
/**
 * The reader beside the message list is folded away. Like the sidebar's
 * rail, this survives a restart: the window comes back as it was left.
 */
readerCollapsed: boolean,
/**
 * Whether to show the list and reader side by side or one at a time.
 */
readingMode: ReadingMode,
/**
 * Width of the assistant's own island beside the mail, `320..=560`.
 */
assistantWidth: number,
/**
 * Display order only; unknown account ids are ignored by the UI.
 */
accountOrder: Array<string>, };

export type LayoutSettingsPatch = { navWidth?: number, navCollapsed?: boolean, listWidth?: number, readerCollapsed?: boolean, readingMode?: ReadingMode, assistantWidth?: number, accountOrder?: Array<string>, };

export type PreviewLines = "none" | "one" | "two" | "three";

export type MailboxLabel = "name" | "icon";

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

export type BadgeSettings = { enabled: boolean,
/**
 * Accounts whose unread mail is left out of the number.
 */
excludedAccounts: Array<string>, };

export type ComposeOpenMode = "window" | "inline";

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
undoSendSeconds: number,
/**
 * Where new mail, replies and continued drafts open. Files written
 * before the setting existed open a composer window.
 */
openMode: ComposeOpenMode, };

export type McpSettings = {
/**
 * Off until the user turns it on.
 */
enabled: boolean, };

export type LanguagePreference = "system" | "zh-CN" | "en";

export type SupportedLocale = "zh-CN" | "en";

export type CloseBehavior = "ask" | "background" | "quit";

export type StartupView = "inbox" | "now";

export type GeneralSettings = { language: LanguagePreference,
/**
 * Download new versions automatically and install on a normal application
 * exit. Update checks always run; the desktop never restarts by itself.
 */
autoUpdate: boolean,
/**
 * Ask until the user chooses whether closing should keep MailX running.
 * The desktop host applies platform and tray availability constraints.
 */
closeBehavior: CloseBehavior,
/**
 * Optional macOS menu bar shortcut; the Dock remains available.
 */
showMenuBarIcon: boolean,
/**
 * Hide the main window only on a registered desktop login launch. The
 * desktop host still requires an available Dock/tray recovery entry.
 */
silentStart: boolean,
/**
 * Single-key mail shortcuts in the main window (`j`, `e`, `s`, ...).
 * Modifier shortcuts such as Cmd/Ctrl+N stay on regardless.
 */
singleKeyShortcuts: boolean,
/**
 * The view shown when the main window starts: the Now page (the default,
 * also for files written before the setting existed) or the unified inbox
 * (now-page.md section 2).
 */
startupView: StartupView, };

export type MailSettings = {
/**
 * After a dedicated verification-code copy succeeds, move that email to
 * the account's Trash. Never authorizes a permanent deletion.
 */
trashVerificationMailAfterCopy: boolean, };

export type AppSettings = { general: GeneralSettings, mail: MailSettings, appearance: AppearanceSettings, security: SecuritySettings, layout: LayoutSettings, notifications: NotificationSettings, badge: BadgeSettings, compose: ComposeSettings, mcp: McpSettings, translation: TranslationSettings, assistant: AssistantSettings, };

export type SettingsPatch = { general?: GeneralSettings, mail?: MailSettings, appearance?: AppearanceSettings, security?: SecuritySettings, layout?: LayoutSettingsPatch, notifications?: NotificationSettings, badge?: BadgeSettings, compose?: ComposeSettings, translation?: TranslationSettings, assistant?: AssistantSettings, };

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

export type MailChangeSource = "user" | "mcp" | "ai";

export type MailStateChangeKind = "flags" | "archive" | "unarchive" | "undoArchive";

export type MailStateChanged = { accountIds: Array<string>, emailIds: Array<string>, source: MailChangeSource, kind: MailStateChangeKind, changedCount: number,
/**
 * A short-lived, one-use capability delivered only to the trusted UI.
 */
undoToken: string | null, };

export type MessagesDeleted = { accountId: string, deleted: number, givenBack: number, };

export type EmailDeletion = { batchId: string,
/**
 * RFC 3339, UTC.
 */
undoUntil: string,
/**
 * `true` when the messages are expunged for good instead of going to
 * Trash (they were in Trash, or the account has none).
 */
permanent: boolean,
/**
 * Ids skipped by a `partial` request, in input order; always empty
 * otherwise.
 */
skippedIds: Array<string>, };

export type ArchiveScope = "exclude" | "only" | "include";

export type ArchiveOutcome = { processedCount: number, changedCount: number,
/**
 * For the trusted host UI only. Tool results must not expose this token.
 */
undoToken: string | null,
/**
 * Ids a `partial` request skipped, in input order; always empty otherwise.
 */
skippedIds: Array<string>, };

export type FlagBatchItemOutcome = "unchanged" | "queued" | "localOnly" | "failed";

export type FlagBatchItem = { emailId: string, outcome: FlagBatchItemOutcome,
/**
 * Set only when `outcome` is `failed`.
 */
code: ErrorCode | null, };

export type FlagBatchOutcome = { results: Array<FlagBatchItem>, };

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
 * All arrivals in this sync round, newest first, so a host can identify
 * verification mail before grouping. Empty when previews are turned off.
 */
messages: Array<NewMailMessage>, };

export type EmailPreviewItem = { emailId: string, preview: string, previewState: PreviewState, };

export type EmailPreviews = { items: Array<EmailPreviewItem>, };

export type SentCopiesChanged = { accountId: string, };

export type AvatarImage = { mimeType: string,
/**
 * Base64 of the re-encoded, validated image.
 */
data: string, kind: AvatarKind, };

export type DraftKind = "new" | "reply" | "replyAll" | "forward";

export type DraftOrigin = "user" | "mcp" | "ai";

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
revision: number, attachments: Array<DraftAttachmentSummary>, origin: DraftOrigin, };

export type DraftSummary = { id: string, accountId: string, kind: DraftKind, sourceEmailId: string | null, to: Array<MailAddress>, subject: string,
/**
 * Start of the body on one line.
 */
preview: string, updatedAt: string, revision: number, origin: DraftOrigin, };

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

export type AddAttachmentRequest = { draftId: string, expectedRevision: number,
/**
 * The name the host got from the user; the core cleans it up.
 */
fileName: string, };

export type RemoveAttachmentRequest = { draftId: string, attachmentId: string, expectedRevision: number, };

export type DraftAttachmentSummary = { id: string, fileName: string, mediaType: string, size: number, origin: AttachmentOrigin,
/**
 * An image shown inside the body through `cid:`; not listed as a file.
 */
inline: boolean,
/**
 * The Content-ID, without angle brackets, that the body's `<img>` uses.
 * Set exactly when `inline` is true.
 */
contentId: string | null, };

export type InlineImage = { attachmentId: string,
/**
 * Without angle brackets.
 */
contentId: string, fileName: string, };

export type AttachmentOrigin = "user" | "forwarded" | "mcp" | "ai";

export type AttachmentPolicy = {
/**
 * Sum of the raw attachment bytes of one draft.
 */
maxTotalBytes: number, maxCount: number,
/**
 * Raw attachment capacity, in whole MB, that the account's SMTP server
 * advertised (`SIZE` / 1.37 for base64 and MIME overhead, rounded down).
 * Only a hint for the composer; `maxTotalBytes` stays the enforced limit.
 * `None` until a send has learned it.
 */
serverLimitMb: number | null, };

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
 * `messageTooLarge` (the server named a size limit and refused, before the
 * body at `MAIL FROM` or in the final reply), `rejectedAfterData`,
 * `retryExhausted`, `accountChanged`, `unknown`.
 */
reason: string, smtpCode: number | null, text: string | null, rejected: Array<RejectedRecipientInfo>, };

export type ServerReplyInfo = {
/**
 * SMTP reply code. EWS acceptance has no SMTP code.
 */
code: number | null, text: string, };

export type OutboxItem = { id: string,
/**
 * Cleared when the draft is deleted (after `accepted`, the item is the
 * local Sent copy and outlives its draft).
 */
draftId: string | null, accountId: string,
/**
 * Sender address frozen in the SMTP envelope when this item was queued.
 */
senderEmailAddress: string,
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

export type TranslationProviderId = "deepl" | "microsoft" | "google" | "microsoftFree" | "googleFree" | "ai";

export type ServiceRoute = { "kind": "direct" } | { "kind": "proxy", proxyKind: ProxyKind, host: string, port: number, username: string, };

export type AiConsent = { connectionId: string, baseUrl: string, };

export type TranslationSettings = {
/**
 * `None` means the user turned translation off; a missing field is the default.
 */
provider: TranslationProviderId | null,
/**
 * Azure resource region; `""` for a global resource.
 */
microsoftRegion: string, route: ServiceRoute,
/**
 * `"auto"` (follow the UI language) or one of `translate::languages::TARGETS`.
 */
targetLanguage: string,
/**
 * Offer to translate when a mail is in another language.
 */
suggest: boolean,
/**
 * Base language codes that never get the offer.
 */
neverSuggest: Array<string>,
/**
 * Base language codes translated without asking.
 */
alwaysTranslate: Array<string>,
/**
 * Translate folded quoted text too.
 */
includeQuotes: boolean, consentedProvider: TranslationProviderId | null,
/**
 * With `consented_provider = ai`: which connection the agreement covers.
 */
consentedAi: AiConsent | null, };

export type TranslationKeyState = { provider: TranslationProviderId, configured: boolean, };

export type TranslationProxyPasswordState = { configured: boolean, };

export type TranslationTest = { translated: string, };

export type TranslationStatus = { emailId: string,
/**
 * Resolved: the explicit target, else the setting, with `auto` following the UI language.
 */
targetLanguage: string,
/**
 * Base code from local detection; `None` when unknown.
 */
detectedLanguage: string | null, detectionReliable: boolean, suggested: boolean, autoTranslate: boolean,
/**
 * Present when a current cached translation exists for the target and quote setting.
 */
translation: TranslationInfo | null, };

export type TranslationInfo = { provider: TranslationProviderId, sourceLanguage: string | null, subject: string | null,
/**
 * Some text was left untranslated (size budget).
 */
partial: boolean, };

export type TranslationCacheStats = { count: number, bytes: number, };

export type AiProtocol = "openaiCompatible" | "anthropic";

export type AiConnection = {
/**
 * Generated by the core.
 */
id: string,
/**
 * A catalog id, or `custom`.
 */
providerId: string, name: string, protocol: AiProtocol,
/**
 * Normalized: no trailing `/`; the version segment is part of it.
 */
baseUrl: string,
/**
 * Goes through the AI proxy when one is set; otherwise connects directly.
 */
useProxy: boolean, };

export type AiModelRef = { connectionId: string, modelId: string, name: string, contextLength: number | null, outputLength: number | null,
/**
 * Whether the model supports tool calls, when the catalog knew. `None`
 * (unknown, and every older settings file) is treated as "try".
 */
toolCall?: boolean, };

export type AiSettings = { connections: Array<AiConnection>, defaultModel: AiModelRef | null,
/**
 * The model that translates mail; `None` follows `default_model`.
 */
translationModel: AiModelRef | null,
/**
 * The AI proxy, shared by every connection that uses it and by the
 * catalog and icon downloads.
 */
route: ServiceRoute, };

export type AiConnectionState = { hasKey: boolean,
/**
 * Generated by the core.
 */
id: string,
/**
 * A catalog id, or `custom`.
 */
providerId: string, name: string, protocol: AiProtocol,
/**
 * Normalized: no trailing `/`; the version segment is part of it.
 */
baseUrl: string,
/**
 * Goes through the AI proxy when one is set; otherwise connects directly.
 */
useProxy: boolean, };

export type AiState = { connections: Array<AiConnectionState>, defaultModel: AiModelRef | null, translationModel: AiModelRef | null, route: ServiceRoute, proxyPasswordSet: boolean, };

export type SaveAiConnection = { id: string | null, providerId: string, name: string, protocol: AiProtocol, baseUrl: string, useProxy: boolean, apiKey: string | null, verify: boolean, };

export type SavedAiConnection = { state: AiState,
/**
 * The saved connection, which is new when the request had no `id`.
 */
connectionId: string,
/**
 * The provider's models when `verify` was set and listing is supported.
 */
models: Array<AiModel> | null, };

export type AiCatalogProvider = { id: string, name: string, protocol: AiProtocol, baseUrl: string, builtIn: boolean,
/**
 * Runs on this computer; no key needed.
 */
local?: boolean, };

export type AiCatalog = {
/**
 * Built-in providers first, then the rest by name. The frontend adds
 * "custom" itself.
 */
providers: Array<AiCatalogProvider>,
/**
 * When the models.dev data was last confirmed current (RFC 3339);
 * `None` without a cache.
 */
updatedAt: string | null,
/**
 * No cache, or one older than 24 hours.
 */
stale: boolean,
/**
 * A refresh was asked for and failed; the cache above is still in use.
 */
refreshError: ErrorCode | null, };

export type AiModelSource = "provider" | "catalog";

export type AiModel = { id: string, name: string, contextLength: number | null, outputLength: number | null,
/**
 * `None` when nobody says.
 */
toolCall: boolean | null,
/**
 * Display-only attribution reported by the list endpoint, never a route.
 */
ownedBy?: string, source: AiModelSource, };

export type AiTestResult = { latencyMs: number, };

export type AiProviderIcon = { mimeType: "image/svg+xml",
/**
 * Base64 of the SVG text.
 */
data: string, };

export type AssistantMode = "direct" | "ask";

export type AssistantSendMode = "confirm" | "direct";

export type SummaryStyle = "brief" | "detailed" | "points";

export type SummaryLanguage = { "mode": "interface" } | { "mode": "original" } | { "mode": "fixed", code: string, };

export type EmailSummarySettings = {
/**
 * Summarize every email the user opens. Off by default: it sends each
 * opened message to the model.
 */
auto: boolean,
/**
 * Lowercased sender addresses whose mail is always summarized, whatever
 * `auto` says.
 */
senders: Array<string>,
/**
 * The style the panel last used.
 */
style: SummaryStyle,
/**
 * The language the panel last used.
 */
language: SummaryLanguage,
/**
 * The summary card shows only its first line.
 */
collapsed: boolean, };

export type AssistantSettings = {
/**
 * Off: no run starts, whatever asks for it.
 */
enabled: boolean,
/**
 * `None` follows `ai.defaultModel`.
 */
model: AiModelRef | null,
/**
 * The connection and base URL the user agreed to send mail content to.
 */
consented: AiConsent | null,
/**
 * Account ids the assistant cannot see.
 */
excludedAccounts: Array<string>,
/**
 * Flags, archive and refresh.
 */
organize: AssistantMode,
/**
 * Changes to drafts the assistant created itself.
 */
aiDrafts: AssistantMode, send: AssistantSendMode,
/**
 * Appended to the system prompt, at most 2000 characters.
 */
instructions: string,
/**
 * Keep chat conversations on this device so they can be reopened.
 * Turning it off stops saving new turns; what is already saved stays
 * until the user deletes it.
 */
saveHistory: boolean,
/**
 * Let the assistant read a mail's HTML structure (headings, lists,
 * tables, links) instead of its plain text only. Off by default: it sends
 * far more of the message, including markup other people wrote.
 */
readHtml: boolean,
/**
 * Email summaries in the reader.
 */
summary: EmailSummarySettings, };

export type StartRun = { kind: RunKind, };

export type RunKind = { "type": "nowSummary", since: string | null, dayStarts?: Array<string>,
/**
 * Skip the cached answer and ask the model again (the refresh
 * button); the new answer replaces the cached one.
 */
fresh: boolean,
/**
 * What today is on the user's calendar, when it is something: the
 * interface works it out the same way as for the preset line. A closed
 * list, never free text, so it cannot carry anything into the prompt.
 */
occasion?: Occasion, } | { "type": "chat", conversationId?: string, message: string,
/**
 * What the user is looking at as they write. The core reads it
 * itself, within the assistant's accounts, for this run only.
 */
context?: ChatContext,
/**
 * Answer the conversation's last user message again in place of its
 * last reply; `message` must be empty and `conversationId` set. Only
 * a reply whose tool calls all read (`Conflict` otherwise).
 */
regenerate?: boolean,
/**
 * Go on with the reply that was cut off (stopped, or at the output
 * limit); `message` must be empty and `conversationId` set. The
 * model is told to continue from where it stopped; no user message
 * is added.
 */
continuation?: boolean,
/**
 * Reword an earlier message: the `edit`-th user message of the
 * conversation (from 0) becomes `message`, and it and everything
 * after it are replaced. Needs `conversationId`; nothing after it
 * may have changed mail (`Conflict` otherwise).
 */
edit?: number, } | { "type": "emailSummary", emailId: string, style?: SummaryStyle, language?: SummaryLanguage,
/**
 * Skip the cached summary and ask the model again; the new one
 * replaces it.
 */
fresh?: boolean, } | { "type": "composeAssist", draftId: string, task: ComposeTask, };

export type ChatContext = { "type": "email", emailId: string, } | { "type": "emails", emailIds: Array<string>, } | { "type": "draft", draftId: string, };

export type ComposeTask = { "type": "draft", instruction: string, language?: string, } | { "type": "rewrite", text: string, style: RewriteStyle, } | { "type": "proofread", text: string, };

export type RewriteStyle = { "type": "polish" } | { "type": "formal" } | { "type": "friendly" } | { "type": "concise" } | { "type": "expand" } | { "type": "translate", code: string, } | { "type": "custom", instruction: string, };

export type ComposeAssistState = {
/**
 * The assistant is on and a model is usable.
 */
usable: boolean,
/**
 * The user agreed to send mail content to that model's connection.
 */
consented: boolean,
/**
 * The connection's name, for the consent question.
 */
service: string, };

export type Occasion = "newYear" | "springFestival" | "lantern" | "dragonBoat" | "qixi" | "midAutumn" | "xiaohan" | "dahan" | "lichun" | "yushui" | "jingzhe" | "chunfen" | "qingming" | "guyu" | "lixia" | "xiaoman" | "mangzhong" | "xiazhi" | "xiaoshu" | "dashu" | "liqiu" | "chushu" | "bailu" | "qiufen" | "hanlu" | "shuangjiang" | "lidong" | "xiaoxue" | "daxue" | "dongzhi";

export type RunKindTag = "nowSummary" | "chat" | "emailSummary" | "composeAssist";

export type CachedEmailSummary = { style: SummaryStyle,
/**
 * A translation target code, or `original` for the email's own language.
 */
language: string,
/**
 * Untrusted model output; shown as Markdown without raw HTML.
 */
text: string,
/**
 * RFC 3339 UTC with milliseconds.
 */
createdAt: string, };

export type RunState = "awaitingConsent" | "running" | "awaitingConfirmation" | "completed" | "failed" | "canceled";

export type FinishedReason = "stop" | "limit" | "length";

export type ToolStatus = "running" | "done" | "failed" | "denied";

export type ToolUndo = { "type": "archive", token: string, } | { "type": "flags", restore: Array<FlagRestore>, };

export type FlagRestore = { emailId: string, isRead: boolean | null, isStarred: boolean | null, };

export type ConfirmAction = "send" | "cancelSend" | "organize" | "aiDraft" | "userDraft";

export type ConfirmWarning = "newRecipient" | "notInConversation" | "hasAttachments";

export type ConfirmAttachment = { fileName: string, size: number, };

export type OrganizeChange = { "type": "archive" } | { "type": "unarchive" } | { "type": "flags", isRead: boolean | null, isStarred: boolean | null, } | { "type": "refresh" };

export type ConfirmDraftChange = { "type": "create" } | { "type": "update", fields: Array<string>, } | { "type": "delete" } | { "type": "attachmentAdded", fileName: string, } | { "type": "attachmentRemoved", fileName: string, };

export type ConfirmationDetails = { "type": "send", accountName: string, accountAddress: string, to: Array<MailAddress>, cc: Array<MailAddress>, bcc: Array<MailAddress>, subject: string,
/**
 * The first characters of the plain-text body.
 */
bodyPreview: string, attachments: Array<ConfirmAttachment>, warnings: Array<ConfirmWarning>, } | { "type": "outbox", retry: boolean, senderAddress: string, subject: string,
/**
 * The first few recipients; `recipientCount` counts all.
 */
recipients: Array<string>, recipientCount: number, } | { "type": "organize", change: OrganizeChange,
/**
 * Emails the call targets that exist and are in scope.
 */
count: number,
/**
 * Subjects of up to five of them.
 */
subjects: Array<string>, } | { "type": "draft", subject: string, origin: DraftOrigin, change: ConfirmDraftChange, };

export type ConfirmationRequest = { action: ConfirmAction,
/**
 * The tool id, e.g. `outbox_send`.
 */
tool: string, details: ConfirmationDetails, };

export type PendingConfirmation = { requestId: string, callId: string, request: ConfirmationRequest, };

export type ToolActivitySummary = { "type": "pending" } | { "type": "found", count: number, } | { "type": "read", emailIds: Array<string>, } | { "type": "changed", count: number, emailIds: Array<string>, undo: ToolUndo | null, } | { "type": "draft", draftId: string, subject: string, } | { "type": "outbox", outboxId: string, } | { "type": "done" } | { "type": "denied" } | { "type": "error", code: string, };

export type ToolActivity = { callId: string, name: string, status: ToolStatus, summary: ToolActivitySummary, };

export type RunReasoning = {
/**
 * The readable reasoning so far; empty when the server only sent
 * redacted thinking.
 */
text: string,
/**
 * RFC 3339 UTC with milliseconds.
 */
startedAt: string,
/**
 * `None` while it is still going.
 */
durationMs: number | null,
/**
 * Where in `text` of the snapshot it began, in UTF-16 code units.
 */
textOffset: number,
/**
 * How many of the run's `tools` began before it.
 */
toolsBefore: number, };

export type RunSnapshot = { runId: string, kind: RunKindTag, state: RunState,
/**
 * Everything the model has written so far.
 */
text: string, tools: Array<ToolActivity>,
/**
 * The model's thinking, in order.
 */
reasoning: Array<RunReasoning>,
/**
 * Whether this run may use tools. `false` for a chat on a model known to
 * lack tool calling (it can only answer from the conversation) and for
 * Now summaries; `true` for every other chat.
 */
toolsAvailable: boolean, error: ErrorCode | null, finishedReason: FinishedReason | null,
/**
 * When the run ended (`completed`, `failed` or `canceled`), RFC 3339 UTC
 * with milliseconds; `None` while it is going. A Now summary answered
 * from the cache carries the time its text was first generated.
 */
completedAt: string | null,
/**
 * The `seq` of the last event this snapshot includes; later events
 * continue from it.
 */
seq: number,
/**
 * Set for chats.
 */
conversationId: string | null,
/**
 * The question the run waits on; set exactly while `state` is
 * `awaitingConfirmation`.
 */
pending: PendingConfirmation | null, };

export type AssistantEvent = { runId: string,
/**
 * Increases by one per event of a run, starting at 1.
 */
seq: number, kind: AssistantEventKind, };

export type AssistantEventKind = { "type": "state", state: RunState, } | { "type": "text", delta: string, } | { "type": "reasoning", segment: number, delta: string, } | { "type": "reasoningEnd", segment: number, durationMs: number, } | { "type": "tool", callId: string, name: string, status: ToolStatus, summary: ToolActivitySummary, } | { "type": "confirm", requestId: string, callId: string, request: ConfirmationRequest, } | { "type": "finished", reason: FinishedReason, } | { "type": "failed", code: ErrorCode, };

export type ConversationSummary = { id: string,
/**
 * The first user message, shortened.
 */
title: string,
/**
 * RFC 3339 UTC with milliseconds: when the last turn was saved.
 */
updatedAt: string,
/**
 * Messages the user sent.
 */
messageCount: number, };

export type ConversationList = {
/**
 * Newest first.
 */
items: Array<ConversationSummary>,
/**
 * Pass back as `cursor` for the next page; `None` at the end.
 */
nextCursor: string | null, };

export type TurnState = "completed" | "failed" | "canceled";

export type ConfirmOutcome = "approved" | "denied" | "expired";

export type ChatTurnTool = { callId: string, name: string, status: ToolStatus,
/**
 * Never carries undo data: `changed.undo` is always `null`.
 */
summary: ToolActivitySummary, textOffset: number, };

export type ChatTurnReasoning = {
/**
 * Empty when the server only sent redacted thinking.
 */
text: string, durationMs: number, textOffset: number,
/**
 * How many of the reply's `tools` began before it.
 */
toolsBefore: number, };

export type ChatTurnConfirmation = { callId: string, tool: string, action: ConfirmAction, outcome: ConfirmOutcome, textOffset: number, };

export type ChatTurn = { "role": "user", text: string,
/**
 * RFC 3339 UTC with milliseconds.
 */
at: string, } | { "role": "assistant", text: string, tools: Array<ChatTurnTool>, reasoning: Array<ChatTurnReasoning>, confirmations: Array<ChatTurnConfirmation>, state: TurnState,
/**
 * Set when `state` is `completed`.
 */
finishedReason: FinishedReason | null,
/**
 * Set when `state` is `failed`.
 */
error: ErrorCode | null, at: string, };

export type ConversationDetail = { id: string, title: string, createdAt: string, updatedAt: string, turns: Array<ChatTurn>, };

export type ConversationChange = "saved" | "deleted" | "cleared";

export type ConversationsChanged = { conversationId: string | null, change: ConversationChange, };
