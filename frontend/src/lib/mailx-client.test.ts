import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { createDesktopClient, MailxClientError } from './mailx-client'
import { emailQuery } from '@/features/mail/format'

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

  it('reads folders and sync states from the host', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: [], requestId: 'request' })
    const client = createDesktopClient()
    await client.listFolders('account')
    await client.listSyncStates()
    expect(invoke).toHaveBeenNthCalledWith(1, 'folders_list', { accountId: 'account' })
    expect(invoke).toHaveBeenNthCalledWith(2, 'sync_states', undefined)
  })

  it('forwards the complete email query and the selected email identifier', async () => {
    const query = emailQuery({ folderRole: 'inbox', unread: true, text: '主题', limit: 100, cursor: 'next-page' })
    const page = { items: [], nextCursor: null, coverage: { headersComplete: false, bodiesComplete: false, syncedAfter: null, syncedBefore: null } }
    vi.mocked(invoke)
      .mockResolvedValueOnce({ ok: true, data: page, requestId: 'list' })
      .mockResolvedValueOnce({ ok: true, data: { summary: { id: 'email-id' } }, requestId: 'detail' })
    const client = createDesktopClient()
    await expect(client.listEmails(query)).resolves.toEqual(page)
    await expect(client.getEmail('email-id')).resolves.toEqual({ summary: { id: 'email-id' } })
    expect(invoke).toHaveBeenNthCalledWith(1, 'emails_list', { query })
    expect(invoke).toHaveBeenNthCalledWith(2, 'emails_get', { emailId: 'email-id' })
  })

  it('never invokes IPC when the desktop host is absent', async () => {
    vi.mocked(isTauri).mockReturnValue(false)
    const client = createDesktopClient()
    expect(client.available).toBe(false)
    await expect(client.listAccounts()).rejects.toBeInstanceOf(MailxClientError)
    await expect(client.listEmails(emailQuery({}))).rejects.toBeInstanceOf(MailxClientError)
    await expect(client.getEmail('email-id')).rejects.toBeInstanceOf(MailxClientError)
    expect(invoke).not.toHaveBeenCalled()
  })
})
