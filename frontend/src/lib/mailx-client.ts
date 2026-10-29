import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import type {
  Account,
  AccountRemoval,
  AccountConnectionVerification,
  ConnectionInput,
  DraftConnectionVerification,
  AccountSyncState,
  AppError,
  AppSettings,
  ApiResult,
  AttachmentPolicy,
  AvatarImage,
  RemoteImageStatus,
  FlagWriteBack,
  CreateAccountRequest,
  CreateDraftFromRequest,
  CreateDraftRequest,
  Draft,
  DraftChanged,
  DraftSummary,
  OutboxChanged,
  OutboxItem,
  QueueSendRequest,
  RemoveAttachmentRequest,
  ResolveUnknownRequest,
  UpdateDraftRequest,
  NotificationSound,
  EmailDetail,
  EmailListQuery,
  EmailPage,
  Folder,
  SettingsSnapshot,
  UpdateConnectionRequest,
  UpdateProfileRequest,
  RemoveAccountRequest,
} from '@/generated/accounts'
import type { StartupStatus } from '@/generated/startup'
import type { UpdateSnapshot } from '@/features/updater/types'

export type ClientFailure = AppError | {
  code: 'hostUnavailable' | 'transport'
  message: string
  field: null
}

export class MailxClientError extends Error {
  readonly failure: ClientFailure

  constructor(failure: ClientFailure) {
    super(failure.message)
    this.name = 'MailxClientError'
    this.failure = failure
  }
}

export interface MailxClient {
  readonly available: boolean
  getStartupStatus(): Promise<StartupStatus>
  retryStartup(): Promise<StartupStatus>
  listAccounts(): Promise<Account[]>
  getAccount(accountId: string): Promise<Account>
  /** Authenticates both saved endpoints without sending mail; abort cancels this attempt only. */
  verifyAccountConnection(accountId: string, expectedRevision: number, signal?: AbortSignal): Promise<AccountConnectionVerification>
  /** Tests the supplied connection without saving an account or sending mail. */
  verifyDraftConnection(connection: ConnectionInput, signal?: AbortSignal): Promise<DraftConnectionVerification>
  /** Opens the fixed, reviewed help page for the selected supported provider. */
  openProviderHelp(providerId: string): Promise<void>
  createAccount(request: CreateAccountRequest): Promise<Account>
  /** Replaces the connection; the account's sync restarts with it right away. */
  updateConnection(request: UpdateConnectionRequest): Promise<Account>
  /** Renames or recolors the account; sync keeps running. */
  updateProfile(request: UpdateProfileRequest): Promise<Account>
  /** Removes local data only. A sending item blocks removal; retries return the same receipt. */
  removeAccount(request: RemoveAccountRequest): Promise<AccountRemoval>
  /** Main and composer windows refresh accounts and retire the exact removed drafts. */
  onAccountRemoved(listener: (removal: AccountRemoval) => void): () => void
  listFolders(accountId: string): Promise<Folder[]>
  listSyncStates(): Promise<AccountSyncState[]>
  /**
   * Changes read / starred in the local copy at once; writing it to the server
   * happens in the background. `localOnly` means the server cannot keep a starred
   * flag in that folder, so the star stays on this device.
   */
  setFlag(emailId: string, flag: 'read' | 'starred', wanted: boolean): Promise<'unchanged' | 'queued' | 'localOnly'>
  /** Hears each finished write-back round: what was confirmed, and stars kept local. */
  onFlagWriteBack(listener: (written: FlagWriteBack) => void): () => void
  /** Asks the account's sync worker to run its next round now. */
  refreshSync(accountId: string): Promise<void>
  /** `null` unless the user enabled sender avatars and the sender has one. */
  getAvatar(accountId: string, address: string): Promise<AvatarImage | null>
  /**
   * Opens the system save dialog and writes the attachment where the user picks.
   * No path crosses this boundary: the page only names the attachment.
   */
  saveAttachment(emailId: string, attachmentId: string): Promise<'saved' | 'cancelled'>
  /** Deletes cached avatars, site icons and downloaded message images. */
  clearImageCache(): Promise<void>
  /** Receives versioned account snapshots, including per-folder progress and cache revisions. */
  onSyncState(listener: (state: AccountSyncState) => void): () => void
  listEmails(query: EmailListQuery): Promise<EmailPage>
  getEmail(emailId: string): Promise<EmailDetail>
  fetchBody(emailId: string): Promise<EmailDetail>
  prepareBody(emailId: string, loadRemoteImages?: boolean): Promise<{ documentToken: string }>
  releaseBody(emailId: string, documentToken: string): Promise<void>
  imageStatus(emailId: string, documentToken: string): Promise<RemoteImageStatus[]>
  retryImages(emailId: string, documentToken: string): Promise<{ documentToken: string }>
  bodyLinks(emailId: string, documentToken: string): Promise<BodyLinks>
  requestLink(request: BodyLinkIdentity): Promise<void>
  openLink(request: BodyLinkIdentity & { requestId: string }): Promise<void>
  onLinkRequest(listener: (request: BodyLinkRequest) => void): () => void
  /** App preferences from the core's `settings.json`; available even when startup failed. */
  getSettings(): Promise<SettingsSnapshot>
  /** Updates the supplied preference sections; the core validates them and returns the canonical form. */
  updateSettings(settings: Partial<AppSettings>): Promise<SettingsSnapshot>
  /** Resolves once the host event listener is registered. */
  onSettingsChanged(listener: (change: { revision: number }) => void): Promise<() => void>
  /** Shows a sample notification with the saved notification settings. */
  testNotification(): Promise<void>
  /** Plays one of MailX's own notification sounds; the system sound and none play nothing. */
  previewSound(sound: NotificationSound): Promise<void>
  /** Hears clicks on new-mail notifications: one message, or a whole account's arrivals. */
  onNotificationOpen(listener: (request: NotificationOpen) => void): () => void

  // --- Drafts and sending (docs/architecture/sending.md). Which window may call
  // what is enforced by the host: the composer window only reaches its own draft.
  /** Main window. Creates a blank draft; the host supplies the default sender account. */
  createDraft(request: CreateDraftRequest): Promise<Draft>
  /** Main window. Reply, reply-all or forward from a message whose body is cached (`bodyNotCached` otherwise). */
  createDraftFrom(request: CreateDraftFromRequest): Promise<Draft>
  listDrafts(accountId?: string | null): Promise<DraftSummary[]>
  getDraft(id: string): Promise<Draft>
  /** Fails with `conflict` unless `expectedRevision` is current; `draftLocked` while a queue item holds the draft. */
  updateDraft(request: UpdateDraftRequest): Promise<Draft>
  deleteDraft(id: string): Promise<void>
  /**
   * Composer window, own draft only. The host opens the native picker and attaches the chosen files; no path
   * reaches the page. Resolves `null` when the picker was cancelled. An error after some files were added leaves
   * those attached, so reload the draft on failure.
   */
  addAttachments(draftId: string, expectedRevision: number): Promise<Draft | null>
  /** Composer window. Attaches the files the OS dropped on this window since `onFilesDropped` fired; `null` if none. */
  addDroppedAttachments(draftId: string, expectedRevision: number): Promise<Draft | null>
  /** Composer window, own draft only. `draftLocked` once the draft is queued. */
  removeAttachment(request: RemoveAttachmentRequest): Promise<Draft>
  /** Limits for drafts of this account; `serverLimitMb` is only a hint, the core stays authoritative. */
  attachmentPolicy(accountId: string): Promise<AttachmentPolicy>
  /** Composer window. Fires when files are dropped on it (the paths stay in the host). */
  onFilesDropped(listener: () => void): () => void
  /** Composer window only, own draft only. Queues the draft; the undo window comes from settings. */
  queueSend(request: QueueSendRequest): Promise<OutboxItem>
  listOutbox(options?: { accountId?: string | null; includeAll?: boolean }): Promise<OutboxItem[]>
  getOutbox(id: string): Promise<OutboxItem>
  /** Undo send; only while still `queued` (`alreadySending` otherwise). */
  cancelOutbox(id: string): Promise<OutboxItem>
  retryOutboxNow(id: string): Promise<OutboxItem>
  /** Settles an `unknown` item; `resend` is an explicit new attempt and returns the new item. */
  resolveUnknown(request: ResolveUnknownRequest): Promise<OutboxItem>
  /** Main window. Opens or focuses the draft's composer window; `fresh` marks a draft created for it. */
  openComposer(draftId: string, fresh?: boolean): Promise<void>
  /** Composer window. Closes its own window; flush the draft first. */
  closeComposer(force?: boolean): Promise<void>
  /** Every queue state change, for the main window and the draft's composer. No addresses or subject. */
  onOutbox(listener: (change: OutboxChanged) => void): () => void
  /** A draft was created, edited, deleted, or locked / released by the queue. */
  onDrafts(listener: (change: DraftChanged) => void): () => void
  /** Composer window. The main window is closing: flush, then call `closeComposer`. The host closes anyway after about 3 s. */
  onAppClosing(listener: () => void): () => void
  /** Composer window. The system asked to close it: flush, then call `closeComposer`. The host closes anyway after about 3 s. */
  onComposerCloseRequested(listener: () => void): () => void

  // --- Updates (docs/architecture/updater.md). Main window only.
  /** The current update state, including whether this edition can install one. */
  getUpdateState(): Promise<UpdateSnapshot>
  /** Checks the release manifest now; a failure comes back as a `failed` snapshot. */
  checkForUpdates(): Promise<UpdateSnapshot>
  /** Starts downloading the version that was found; progress arrives through `onUpdateState`. */
  downloadUpdate(): Promise<UpdateSnapshot>
  /** Stops a download in progress and returns to `available`. */
  cancelUpdate(): Promise<UpdateSnapshot>
  /** Quits gracefully, installs, and relaunches. Refused (`alreadySending`) while mail is being sent. */
  restartToUpdate(): Promise<UpdateSnapshot>
  /** Opens a release-notes link in the system browser; the host accepts `https:` only. */
  openUpdateLink(url: string): Promise<void>
  /** Every change of the update state. */
  onUpdateState(listener: (snapshot: UpdateSnapshot) => void): () => void
}

export type NotificationOpen = { accountId: string; emailId: string | null; outboxId?: string | null }

/** Host events (`mailx://...`) that the clients subscribe to. */
export const HOST_EVENTS = {
  outbox: 'mailx://outbox',
  drafts: 'mailx://drafts',
  appClosing: 'mailx://app-closing',
  composerCloseRequested: 'mailx://composer-close-requested',
  filesDropped: 'composer://files-dropped',
  settingsChanged: 'mailx://settings-changed',
  updater: 'mailx://updater-state',
} as const

export type BodyLinkIdentity = { emailId: string; index: number; documentToken: string }
export type BodyLinkRequest = BodyLinkIdentity & { url: string; requestId: string }
export type BodyLinks = { documentToken: string; links: { emailId: string; index: number; url: string }[] }

const SYNC_STATE_EVENT = 'mailx://sync-state'

export function clientFailure(error: unknown): ClientFailure {
  if (error instanceof MailxClientError) return error.failure
  // Never display raw IPC exceptions: they may contain host paths or input values.
  return { code: 'transport', message: '与桌面应用的通信中断，请重新加载配置后重试。', field: null }
}

/** The machine-readable code of a failure, for deciding what to do rather than what to say. */
export function clientFailureCode(error: unknown): ClientFailure['code'] {
  return clientFailure(error).code
}

export function createDesktopClient(): MailxClient {
  const available = isTauri()
  async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    if (!available) {
      throw new MailxClientError({ code: 'hostUnavailable', message: '请在 MailX 桌面应用中管理账号。', field: null })
    }
    let result: ApiResult<T>
    try {
      result = await invoke<ApiResult<T>>(command, args)
    } catch {
      throw new MailxClientError(clientFailure(null))
    }
    if (result.ok === false) throw new MailxClientError(result.error)
    return result.data
  }

  function subscribe<T>(event: string, listener: (payload: T) => void): () => void {
    if (!available) return () => undefined
    let disposed = false
    let unlisten: (() => void) | undefined
    getCurrentWebviewWindow()
      .listen<T>(event, (e) => { if (!disposed) listener(e.payload) })
      .then((stop) => (disposed ? stop() : (unlisten = stop)), () => undefined)
    return () => { disposed = true; unlisten?.() }
  }

  return {
    available,
    getStartupStatus: () => call<StartupStatus>('app_get_startup_status'),
    retryStartup: () => call<StartupStatus>('app_retry_startup'),
    listAccounts: () => call<Account[]>('accounts_list'),
    getAccount: (accountId) => call<Account>('accounts_get', { accountId }),
    async verifyAccountConnection(accountId, expectedRevision, signal) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
      const verificationId = crypto.randomUUID()
      const abort = () => { void call<void>('accounts_cancel_verification', { request: { accountId, verificationId } }).catch(() => undefined) }
      signal?.addEventListener('abort', abort, { once: true })
      try {
        const result = await call<AccountConnectionVerification>('accounts_verify_connection', { request: { accountId, expectedRevision, verificationId } })
        if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
        return result
      } finally {
        signal?.removeEventListener('abort', abort)
      }
    },
    async verifyDraftConnection(connection, signal) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
      const verificationId = crypto.randomUUID()
      const abort = () => { void call<void>('accounts_cancel_draft_verification', { request: { verificationId } }).catch(() => undefined) }
      signal?.addEventListener('abort', abort, { once: true })
      try {
        const result = await call<DraftConnectionVerification>('accounts_verify_draft_connection', { request: { verificationId, connection } })
        if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
        if (result.verificationId !== verificationId) throw new MailxClientError(clientFailure(null))
        return result
      } finally {
        signal?.removeEventListener('abort', abort)
      }
    },
    openProviderHelp: (providerId) => call<void>('accounts_open_provider_help', { providerId }),
    createAccount: (request) => call<Account>('accounts_create', { request }),
    updateConnection: (request) => call<Account>('accounts_update_connection', { request }),
    updateProfile: (request) => call<Account>('accounts_update_profile', { request }),
    removeAccount: (request) => call<AccountRemoval>('accounts_remove', { request }),
    onAccountRemoved: (listener) => subscribe<AccountRemoval>('mailx://account-removed', listener),
    listFolders: (accountId) => call<Folder[]>('folders_list', { accountId }),
    listSyncStates: () => call<AccountSyncState[]>('sync_states'),
    setFlag: (emailId, flag, wanted) => call('emails_set_flag', { emailId, flag, wanted }),
    onFlagWriteBack(listener) {
      if (!available) return () => undefined
      let disposed = false
      let unlisten: (() => void) | undefined
      getCurrentWebviewWindow()
        .listen<FlagWriteBack>('mailx://flags', (event) => { if (!disposed) listener(event.payload) })
        .then((stop) => (disposed ? stop() : (unlisten = stop)), () => undefined)
      return () => { disposed = true; unlisten?.() }
    },
    refreshSync: (accountId) => call<void>('sync_refresh', { accountId }),
    getAvatar: (accountId, address) => call<AvatarImage | null>('avatars_get', { accountId, address }),
    saveAttachment: (emailId, attachmentId) => call<'saved' | 'cancelled'>('attachments_save', { emailId, attachmentId }),
    clearImageCache: () => call<void>('image_cache_clear'),
    onSyncState(listener) {
      if (!available) return () => undefined
      let disposed = false
      let unlisten: (() => void) | undefined
      getCurrentWebviewWindow()
        .listen<AccountSyncState>(SYNC_STATE_EVENT, (event) => listener(event.payload))
        .then((stop) => (disposed ? stop() : (unlisten = stop)), () => undefined)
      return () => {
        disposed = true
        unlisten?.()
      }
    },
    listEmails: (query) => call<EmailPage>('emails_list', { query }),
    getEmail: (emailId) => call<EmailDetail>('emails_get', { emailId }),
    fetchBody: (emailId) => call<EmailDetail>('emails_fetch_body', { emailId }),
    prepareBody: (emailId, loadRemoteImages = false) => call('emails_prepare_body', { emailId, loadRemoteImages }),
    releaseBody: (emailId, documentToken) => call('emails_release_body', { emailId, documentToken }),
    imageStatus: (emailId, documentToken) => call('emails_image_status', { emailId, documentToken }),
    retryImages: (emailId, documentToken) => call('emails_retry_images', { emailId, documentToken }),
    bodyLinks: (emailId, documentToken) => call('emails_body_links', { emailId, documentToken }),
    requestLink: ({ emailId, index, documentToken }) => call('links_request', { emailId, index, documentToken }),
    openLink: ({ emailId, index, documentToken, requestId }) => call('links_open', { emailId, index, documentToken, requestId }),
    onLinkRequest(listener) {
      if (!available) return () => undefined
      let disposed = false
      let unlisten: (() => void) | undefined
      getCurrentWebviewWindow()
        .listen<BodyLinkRequest>('mailx://link-request', (event) => { if (!disposed) listener(event.payload) })
        .then((stop) => (disposed ? stop() : (unlisten = stop)), () => undefined)
      return () => { disposed = true; unlisten?.() }
    },
    getSettings: () => call<SettingsSnapshot>('settings_get'),
    async onSettingsChanged(listener) {
      if (!available) return () => undefined
      return getCurrentWebviewWindow().listen<{ revision: number }>(HOST_EVENTS.settingsChanged, (event) => listener(event.payload))
    },
    updateSettings: (request) => call<SettingsSnapshot>('settings_update', { request }),
    testNotification: () => call<void>('notifications_test'),
    previewSound: (sound) => call<void>('notifications_preview_sound', { sound }),
    onNotificationOpen(listener) {
      if (!available) return () => undefined
      let disposed = false
      let unlisten: (() => void) | undefined
      getCurrentWebviewWindow()
        .listen<NotificationOpen>('mailx://notification-open', (event) => { if (!disposed) listener(event.payload) })
        .then((stop) => (disposed ? stop() : (unlisten = stop)), () => undefined)
      return () => { disposed = true; unlisten?.() }
    },
    createDraft: (request) => call<Draft>('drafts_create', { request }),
    createDraftFrom: (request) => call<Draft>('drafts_create_from', { request }),
    listDrafts: (accountId = null) => call<DraftSummary[]>('drafts_list', { accountId }),
    getDraft: (id) => call<Draft>('drafts_get', { id }),
    updateDraft: (request) => call<Draft>('drafts_update', { request }),
    deleteDraft: (id) => call<void>('drafts_delete', { id }),
    addAttachments: (draftId, expectedRevision) => call<Draft | null>('attachments_add', { request: { draftId, expectedRevision } }),
    addDroppedAttachments: (draftId, expectedRevision) =>
      call<Draft | null>('attachments_add_dropped', { request: { draftId, expectedRevision } }),
    removeAttachment: (request) => call<Draft>('attachments_remove', { request }),
    attachmentPolicy: (accountId) => call<AttachmentPolicy>('attachments_policy', { accountId }),
    onFilesDropped: (listener) => subscribe<void>(HOST_EVENTS.filesDropped, () => listener()),
    queueSend: ({ draftId, expectedRevision, clientMutationId }) =>
      call<OutboxItem>('outbox_queue_send', { draftId, expectedRevision, clientMutationId }),
    listOutbox: ({ accountId = null, includeAll = false } = {}) => call<OutboxItem[]>('outbox_list', { accountId, includeAll }),
    getOutbox: (id) => call<OutboxItem>('outbox_get', { id }),
    cancelOutbox: (id) => call<OutboxItem>('outbox_cancel', { id }),
    retryOutboxNow: (id) => call<OutboxItem>('outbox_retry_now', { id }),
    resolveUnknown: ({ id, resolution, clientMutationId }) =>
      call<OutboxItem>('outbox_resolve_unknown', { id, choice: resolution, clientMutationId }),
    openComposer: (draftId, fresh = false) => call<void>('composer_open', fresh ? { draftId, fresh } : { draftId }),
    closeComposer: (force = false) => call<void>('composer_close', { force }),
    onOutbox: (listener) => subscribe<OutboxChanged>(HOST_EVENTS.outbox, listener),
    onDrafts: (listener) => subscribe<DraftChanged>(HOST_EVENTS.drafts, listener),
    onAppClosing: (listener) => subscribe<void>(HOST_EVENTS.appClosing, () => listener()),
    onComposerCloseRequested: (listener) => subscribe<void>(HOST_EVENTS.composerCloseRequested, () => listener()),
    getUpdateState: () => call<UpdateSnapshot>('updater_get_state'),
    checkForUpdates: () => call<UpdateSnapshot>('updater_check'),
    downloadUpdate: () => call<UpdateSnapshot>('updater_download'),
    cancelUpdate: () => call<UpdateSnapshot>('updater_cancel'),
    restartToUpdate: () => call<UpdateSnapshot>('updater_restart'),
    openUpdateLink: (url) => call<void>('updater_open_link', { url }),
    onUpdateState: (listener) => subscribe<UpdateSnapshot>(HOST_EVENTS.updater, listener),
  }
}
