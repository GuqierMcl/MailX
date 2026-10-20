import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import type {
  Account,
  AccountSyncState,
  AppError,
  AppSettings,
  ApiResult,
  CreateAccountRequest,
  EmailDetail,
  EmailListQuery,
  EmailPage,
  Folder,
  SettingsSnapshot,
  UpdateAccountRequest,
} from '@/generated/accounts'
import type { StartupStatus } from '@/generated/startup'

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
  createAccount(request: CreateAccountRequest): Promise<Account>
  updateAccount(request: UpdateAccountRequest): Promise<Account>
  listFolders(accountId: string): Promise<Folder[]>
  listSyncStates(): Promise<AccountSyncState[]>
  /** Receives versioned account snapshots, including per-folder progress and cache revisions. */
  onSyncState(listener: (state: AccountSyncState) => void): () => void
  listEmails(query: EmailListQuery): Promise<EmailPage>
  getEmail(emailId: string): Promise<EmailDetail>
  fetchBody(emailId: string): Promise<EmailDetail>
  prepareBody(emailId: string): Promise<{ documentToken: string }>
  bodyLinks(emailId: string, documentToken: string): Promise<BodyLinks>
  requestLink(request: BodyLinkIdentity): Promise<void>
  openLink(request: BodyLinkIdentity & { requestId: string }): Promise<void>
  onLinkRequest(listener: (request: BodyLinkRequest) => void): () => void
  /** App preferences from the core's `settings.json`; available even when startup failed. */
  getSettings(): Promise<SettingsSnapshot>
  /** Replaces all preferences; the core validates them and returns the canonical form. */
  updateSettings(settings: AppSettings): Promise<SettingsSnapshot>
}

export type BodyLinkIdentity = { emailId: string; index: number; documentToken: string }
export type BodyLinkRequest = BodyLinkIdentity & { url: string; requestId: string }
export type BodyLinks = { documentToken: string; links: { emailId: string; index: number; url: string }[] }

const SYNC_STATE_EVENT = 'mailx://sync-state'

export function clientFailure(error: unknown): ClientFailure {
  if (error instanceof MailxClientError) return error.failure
  // Never display raw IPC exceptions: they may contain host paths or input values.
  return { code: 'transport', message: '与桌面应用的通信中断，请重新加载配置后重试。', field: null }
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

  return {
    available,
    getStartupStatus: () => call<StartupStatus>('app_get_startup_status'),
    retryStartup: () => call<StartupStatus>('app_retry_startup'),
    listAccounts: () => call<Account[]>('accounts_list'),
    getAccount: (accountId) => call<Account>('accounts_get', { accountId }),
    createAccount: (request) => call<Account>('accounts_create', { request }),
    updateAccount: (request) => call<Account>('accounts_update', { request }),
    listFolders: (accountId) => call<Folder[]>('folders_list', { accountId }),
    listSyncStates: () => call<AccountSyncState[]>('sync_states'),
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
    prepareBody: (emailId) => call('emails_prepare_body', { emailId }),
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
    updateSettings: (request) => call<SettingsSnapshot>('settings_update', { request }),
  }
}
