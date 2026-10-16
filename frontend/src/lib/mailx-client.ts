import { invoke, isTauri } from '@tauri-apps/api/core'
import type {
  Account,
  AppError,
  ApiResult,
  CreateAccountRequest,
  UpdateAccountRequest,
} from '@/generated/accounts'

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
  listAccounts(): Promise<Account[]>
  getAccount(accountId: string): Promise<Account>
  createAccount(request: CreateAccountRequest): Promise<Account>
  updateAccount(request: UpdateAccountRequest): Promise<Account>
}

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
    listAccounts: () => call<Account[]>('accounts_list'),
    getAccount: (accountId) => call<Account>('accounts_get', { accountId }),
    createAccount: (request) => call<Account>('accounts_create', { request }),
    updateAccount: (request) => call<Account>('accounts_update', { request }),
  }
}
