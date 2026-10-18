import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { createDesktopClient, MailxClientError, UNSYNCED_PAGE } from './mailx-client'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }))

beforeEach(() => { vi.clearAllMocks(); vi.mocked(isTauri).mockReturnValue(true) })

describe('desktop client boundary', () => {
  it('unwraps results and forwards only the selected account identifier', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: { id: 'test' }, requestId: 'request' })
    expect(await createDesktopClient().getAccount('test')).toEqual({ id: 'test' })
    expect(invoke).toHaveBeenCalledWith('accounts_get', { accountId: 'test' })
  })

  it('exposes startup status commands without accepting client-side readiness', async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce({ ok: true, data: { state: 'initializing', revision: 1 }, requestId: 'request-1' })
      .mockResolvedValueOnce({ ok: true, data: { state: 'ready', revision: 2 }, requestId: 'request-2' })
    const client = createDesktopClient()
    await expect(client.getStartupStatus()).resolves.toEqual({ state: 'initializing', revision: 1 })
    await expect(client.retryStartup()).resolves.toEqual({ state: 'ready', revision: 2 })
    expect(invoke).toHaveBeenNthCalledWith(1, 'app_get_startup_status', undefined)
    expect(invoke).toHaveBeenNthCalledWith(2, 'app_retry_startup', undefined)
  })

  it('preserves sanitized service errors but does not reveal raw IPC errors', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ ok: false, error: { code: 'secretStoreLocked', message: '系统凭据库已锁定。', field: null }, requestId: 'request' })
    const client = createDesktopClient()
    await expect(client.listAccounts()).rejects.toMatchObject({ failure: { code: 'secretStoreLocked' } })
    vi.mocked(invoke).mockRejectedValueOnce(new Error('secret-value /private/user/data/db.sqlite'))
    await expect(client.listAccounts()).rejects.toThrow('与桌面应用的通信中断')
  })

  it('reads folders and sync states from the host but reports mail as not yet synced', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: [], requestId: 'request' })
    const client = createDesktopClient()
    await client.listFolders('account')
    await client.listSyncStates()
    expect(invoke).toHaveBeenNthCalledWith(1, 'folders_list', { accountId: 'account' })
    expect(invoke).toHaveBeenNthCalledWith(2, 'sync_states', undefined)
    vi.mocked(invoke).mockClear()
    await expect(client.listEmails({} as never)).resolves.toEqual(UNSYNCED_PAGE)
    expect(invoke).not.toHaveBeenCalled()
  })

  it('never invokes IPC when the desktop host is absent', async () => {
    vi.mocked(isTauri).mockReturnValue(false)
    const client = createDesktopClient()
    expect(client.available).toBe(false)
    await expect(client.listAccounts()).rejects.toBeInstanceOf(MailxClientError)
    expect(invoke).not.toHaveBeenCalled()
  })
})
