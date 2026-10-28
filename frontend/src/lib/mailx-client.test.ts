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

  it('fetches and prepares bodies, and confirms only the bound link identity without forwarding a URL', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: null, requestId: 'response' })
    const client = createDesktopClient()
    await client.fetchBody('email')
    await client.prepareBody('email')
    await client.imageStatus('email', 'document')
    await client.retryImages('email', 'document')
    await client.bodyLinks('email', 'document')
    const request = { emailId: 'email', index: 3, documentToken: 'document', requestId: 'grant', url: 'https://ignored.example' }
    await client.requestLink(request)
    await client.openLink(request)
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ['emails_fetch_body', { emailId: 'email' }],
      ['emails_prepare_body', { emailId: 'email', loadRemoteImages: false }],
      ['emails_image_status', { emailId: 'email', documentToken: 'document' }],
      ['emails_retry_images', { emailId: 'email', documentToken: 'document' }],
      ['emails_body_links', { emailId: 'email', documentToken: 'document' }],
      ['links_request', { emailId: 'email', index: 3, documentToken: 'document' }],
      ['links_open', { emailId: 'email', index: 3, documentToken: 'document', requestId: 'grant' }],
    ])
  })
})

describe('drafts and sending commands', () => {
  const ok = (data: unknown) => ({ ok: true, data, requestId: 'r' })

  it('forwards each command with only its named arguments', async () => {
    vi.mocked(invoke).mockResolvedValue(ok(null))
    const client = createDesktopClient()
    await client.createDraft({ accountId: null, to: [], cc: [], bcc: [], subject: '', bodyHtml: '', clientMutationId: 'm' })
    await client.createDraftFrom({ emailId: 'e', kind: 'reply', clientMutationId: 'm' })
    await client.listDrafts()
    await client.listDrafts('a')
    await client.getDraft('d')
    await client.deleteDraft('d')
    await client.queueSend({ draftId: 'd', expectedRevision: 2, clientMutationId: 'q' })
    await client.listOutbox()
    await client.listOutbox({ accountId: 'a', includeAll: true })
    await client.getOutbox('o')
    await client.cancelOutbox('o')
    await client.retryOutboxNow('o')
    await client.resolveUnknown({ id: 'o', resolution: 'markSent', clientMutationId: 'c' })
    await client.openComposer('d')
    await client.closeComposer()
    const calls = vi.mocked(invoke).mock.calls
    expect(calls.map(([command, args]) => [command, args])).toEqual([
      ['drafts_create', { request: { accountId: null, to: [], cc: [], bcc: [], subject: '', bodyHtml: '', clientMutationId: 'm' } }],
      ['drafts_create_from', { request: { emailId: 'e', kind: 'reply', clientMutationId: 'm' } }],
      ['drafts_list', { accountId: null }],
      ['drafts_list', { accountId: 'a' }],
      ['drafts_get', { id: 'd' }],
      ['drafts_delete', { id: 'd' }],
      ['outbox_queue_send', { draftId: 'd', expectedRevision: 2, clientMutationId: 'q' }],
      ['outbox_list', { accountId: null, includeAll: false }],
      ['outbox_list', { accountId: 'a', includeAll: true }],
      ['outbox_get', { id: 'o' }],
      ['outbox_cancel', { id: 'o' }],
      ['outbox_retry_now', { id: 'o' }],
      ['outbox_resolve_unknown', { id: 'o', choice: 'markSent', clientMutationId: 'c' }],
      ['composer_open', { draftId: 'd' }],
      ['composer_close', { force: false }],
    ])
  })

  it('never lets the page pick the principal when queueing', async () => {
    vi.mocked(invoke).mockResolvedValue(ok(null))
    const forged = { draftId: 'd', expectedRevision: 1, clientMutationId: 'q', caller: 'composer' }
    await createDesktopClient().queueSend(forged)
    expect(invoke).toHaveBeenCalledWith('outbox_queue_send', { draftId: 'd', expectedRevision: 1, clientMutationId: 'q' })
  })

  it('reports host-refused calls without raw details', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ ok: false, error: { code: 'validation', message: '请求无效。', field: 'caller' }, requestId: 'r' })
    await expect(createDesktopClient().queueSend({ draftId: 'd', expectedRevision: 1, clientMutationId: 'q' })).rejects.toMatchObject({ failure: { field: 'caller' } })
  })

  it('does nothing without the desktop host', async () => {
    vi.mocked(isTauri).mockReturnValue(false)
    const client = createDesktopClient()
    expect(client.onOutbox(() => undefined)).toBeTypeOf('function')
    expect(client.onAppClosing(() => undefined)).toBeTypeOf('function')
    await expect(client.openComposer('d')).rejects.toMatchObject({ failure: { code: 'hostUnavailable' } })
  })
})

describe('saved connection verification', () => {
  it('binds cancellation to the unique attempt and removes the abort listener', async () => {
    const report = { accountId: 'account', revision: 3, imap: { status: 'verified', stage: 'complete', code: null }, smtp: { status: 'verified', stage: 'complete', code: null } }
    let complete!: (value: unknown) => void
    vi.mocked(invoke).mockImplementation((command) => command === 'accounts_verify_connection'
      ? new Promise((resolve) => { complete = resolve })
      : Promise.resolve({ ok: true, data: null, requestId: 'cancel' }))
    const controller = new AbortController()
    const pending = createDesktopClient().verifyAccountConnection('account', 3, controller.signal)
    const args = vi.mocked(invoke).mock.calls[0][1] as { request: { verificationId: string } }
    controller.abort()
    expect(invoke).toHaveBeenCalledWith('accounts_cancel_verification', { request: { accountId: 'account', verificationId: args.request.verificationId } })
    complete({ ok: true, data: report, requestId: 'verify' })
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
    expect(args.request.verificationId).toMatch(/^[0-9a-f-]{36}$/)
  })

  it('does not begin an already cancelled verification and forwards help by provider only', async () => {
    const controller = new AbortController()
    controller.abort()
    const client = createDesktopClient()
    await expect(client.verifyAccountConnection('account', 2, controller.signal)).rejects.toMatchObject({ name: 'AbortError' })
    expect(invoke).not.toHaveBeenCalled()
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: null, requestId: 'help' })
    await client.openProviderHelp('qq')
    expect(invoke).toHaveBeenCalledWith('accounts_open_provider_help', { providerId: 'qq' })
  })
})
