import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { createDesktopClient, MailxClientError } from './mailx-client'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }))

beforeEach(() => { vi.clearAllMocks(); vi.mocked(isTauri).mockReturnValue(true) })

describe('desktop client boundary', () => {
  it('unwraps results and forwards only the selected account identifier', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: { id: 'test' }, requestId: 'request' })
    expect(await createDesktopClient().getAccount('test')).toEqual({ id: 'test' })
    expect(invoke).toHaveBeenCalledWith('accounts_get', { accountId: 'test' })
  })

  it('preserves sanitized service errors but does not reveal raw IPC errors', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ ok: false, error: { code: 'secretStoreLocked', message: '系统凭据库已锁定。', field: null }, requestId: 'request' })
    const client = createDesktopClient()
    await expect(client.listAccounts()).rejects.toMatchObject({ failure: { code: 'secretStoreLocked' } })
    vi.mocked(invoke).mockRejectedValueOnce(new Error('secret-value /private/user/data/db.sqlite'))
    await expect(client.listAccounts()).rejects.toThrow('与桌面应用的通信中断')
  })

  it('never invokes IPC when the desktop host is absent', async () => {
    vi.mocked(isTauri).mockReturnValue(false)
    const client = createDesktopClient()
    expect(client.available).toBe(false)
    await expect(client.listAccounts()).rejects.toBeInstanceOf(MailxClientError)
    expect(invoke).not.toHaveBeenCalled()
  })
})
