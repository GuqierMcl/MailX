import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import type {
  Account,
  AccountSyncState,
  AppError,
  ApiResult,
  CreateAccountRequest,
  EmailDetail,
  EmailListQuery,
  EmailPage,
  Folder,
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
}

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
  }
}
