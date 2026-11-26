import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { createDesktopClient, MailxClientError } from './mailx-client'
import { emailQuery } from '@/features/mail/format'
import type { ConnectionInput } from '@/generated/accounts'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }))

beforeEach(() => { vi.clearAllMocks(); vi.mocked(isTauri).mockReturnValue(true) })

describe('desktop client boundary', () => {
  it('forwards MCP adapter IDs and explicit replacement without accepting file paths', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: null, requestId: 'request' })
    const client = createDesktopClient()
    await client.getMcpAgents()
    await client.setupMcpAgent('openCode')
    await client.setupMcpAgent('codex', true)
    await client.removeMcpAgent('openCode')
    await client.diagnoseMcp()
    expect(invoke).toHaveBeenNthCalledWith(1, 'mcp_agents', undefined)
    expect(invoke).toHaveBeenNthCalledWith(2, 'mcp_agent_setup', { id: 'openCode', replace: false })
    expect(invoke).toHaveBeenNthCalledWith(3, 'mcp_agent_setup', { id: 'codex', replace: true })
    expect(invoke).toHaveBeenNthCalledWith(4, 'mcp_agent_remove', { id: 'openCode' })
    expect(invoke).toHaveBeenNthCalledWith(5, 'mcp_diagnose', undefined)
  })

  it('manages login startup through host commands with no caller-supplied paths', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: { enabled: true, available: true }, requestId: 'request' })
    const client = createDesktopClient()
    await expect(client.getAutostart()).resolves.toEqual({ enabled: true, available: true })
    await client.setAutostart(true)
    expect(invoke).toHaveBeenNthCalledWith(1, 'autostart_get', undefined)
    expect(invoke).toHaveBeenNthCalledWith(2, 'autostart_set', { enabled: true })
  })

  it('undoes only the archive receipt issued by the host', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: null, requestId: 'request' })
    await createDesktopClient().undoArchive('receipt-1')
    expect(invoke).toHaveBeenCalledWith('emails_undo_archive', { undoToken: 'receipt-1' })
  })

  it('removes only the named account at the confirmed revision and returns its receipt', async () => {
    const data = { accountId: 'account', draftIds: ['draft'], credentialsPending: true }
    vi.mocked(invoke).mockResolvedValue({ ok: true, data, requestId: 'request' })
    await expect(createDesktopClient().removeAccount({ accountId: 'account', expectedRevision: 3 })).resolves.toEqual(data)
    expect(invoke).toHaveBeenCalledWith('accounts_remove', { request: { accountId: 'account', expectedRevision: 3 } })
  })
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
      .mockResolvedValueOnce({ ok: true, data: { messages: [], possiblyIncomplete: false }, requestId: 'thread' })
    const client = createDesktopClient()
    await expect(client.listEmails(query)).resolves.toEqual(page)
    await expect(client.getEmail('email-id')).resolves.toEqual({ summary: { id: 'email-id' } })
    expect(invoke).toHaveBeenNthCalledWith(1, 'emails_list', { query })
    expect(invoke).toHaveBeenNthCalledWith(2, 'emails_get', { emailId: 'email-id' })
    await expect(client.getConversation('email-id')).resolves.toEqual({ messages: [], possiblyIncomplete: false })
    expect(invoke).toHaveBeenNthCalledWith(3, 'emails_get_conversation', { emailId: 'email-id' })
  })

  it('forwards the overview query to its own command', async () => {
    const overview = { newMail: { total: 0, accounts: [], latest: [], senders: [] }, starredUnread: 0, unreadSenders: [], verificationCodes: [], recentAttachments: [], generatedAt: '2026-11-14T12:00:00.000Z' }
    vi.mocked(invoke).mockResolvedValueOnce({ ok: true, data: overview, requestId: 'overview' })
    const client = createDesktopClient()
    await expect(client.getMailOverview({ since: '2026-11-14T00:00:00.000Z' })).resolves.toEqual(overview)
    expect(invoke).toHaveBeenCalledWith('emails_overview', { query: { since: '2026-11-14T00:00:00.000Z' } })
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
    await client.requestPreviews(['email', 'other'])
    await client.prepareBody('email')
    await client.imageStatus('email', 'document')
    await client.retryImages('email', 'document')
    await client.bodyLinks('email', 'document')
    const request = { emailId: 'email', index: 3, documentToken: 'document', requestId: 'grant', url: 'https://ignored.example' }
    await client.requestLink(request)
    await client.openLink(request)
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ['emails_fetch_body', { emailId: 'email' }],
      ['emails_request_previews', { emailIds: ['email', 'other'] }],
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
    await client.prepareDraftBody('d', 7)
    await client.deleteDraft('d')
    await client.queueSend({ draftId: 'd', expectedRevision: 2, clientMutationId: 'q' })
    await client.listOutbox()
    await client.listOutbox({ accountId: 'a', includeAll: true })
    await client.getOutbox('o')
    await client.cancelOutbox('o')
    await client.retryOutboxNow('o')
    await client.resolveUnknown({ id: 'o', resolution: 'markSent', clientMutationId: 'c' })
    await client.openComposer('d')
    await client.focusComposer('d')
    await client.setInlineComposer('d')
    await client.setInlineComposer(null)
    await client.closeComposer()
    const calls = vi.mocked(invoke).mock.calls
    expect(calls.map(([command, args]) => [command, args])).toEqual([
      ['drafts_create', { request: { accountId: null, to: [], cc: [], bcc: [], subject: '', bodyHtml: '', clientMutationId: 'm' } }],
      ['drafts_create_from', { request: { emailId: 'e', kind: 'reply', clientMutationId: 'm' } }],
      ['drafts_list', { accountId: null }],
      ['drafts_list', { accountId: 'a' }],
      ['drafts_get', { id: 'd' }],
      ['drafts_prepare_body', { draftId: 'd', expectedRevision: 7 }],
      ['drafts_delete', { id: 'd' }],
      ['outbox_queue_send', { draftId: 'd', expectedRevision: 2, clientMutationId: 'q' }],
      ['outbox_list', { accountId: null, includeAll: false }],
      ['outbox_list', { accountId: 'a', includeAll: true }],
      ['outbox_get', { id: 'o' }],
      ['outbox_cancel', { id: 'o' }],
      ['outbox_retry_now', { id: 'o' }],
      ['outbox_resolve_unknown', { id: 'o', choice: 'markSent', clientMutationId: 'c' }],
      ['composer_open', { draftId: 'd' }],
      ['composer_focus', { draftId: 'd' }],
      ['composer_set_inline', { draftId: 'd' }],
      ['composer_set_inline', { draftId: null }],
      ['composer_close', { force: false, generation: null }],
    ])
  })

  it('reports whether a composer window had the draft', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(ok(true)).mockResolvedValueOnce(ok(false))
    const client = createDesktopClient()
    await expect(client.focusComposer('d')).resolves.toBe(true)
    await expect(client.focusComposer('d')).resolves.toBe(false)
  })

  it('never lets the page pick the principal when queueing', async () => {
    vi.mocked(invoke).mockResolvedValue(ok(null))
    const forged = { draftId: 'd', expectedRevision: 1, clientMutationId: 'q', caller: 'composer' }
    await createDesktopClient().queueSend(forged)
    expect(invoke).toHaveBeenCalledWith('outbox_queue_send', { draftId: 'd', expectedRevision: 1, clientMutationId: 'q' })
  })

  it('binds composer acknowledgements and cancellations to the host close generation', async () => {
    vi.mocked(invoke).mockResolvedValue(ok(null))
    const client = createDesktopClient()
    await client.closeComposer(false, 17)
    await client.cancelComposerClose(18)
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ['composer_close', { force: false, generation: 17 }],
      ['composer_cancel_close', { generation: 18 }],
    ])
  })

  it('reports host-refused calls without raw details', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ ok: false, error: { code: 'validation', message: '请求无效。', field: 'caller' }, requestId: 'r' })
    await expect(createDesktopClient().queueSend({ draftId: 'd', expectedRevision: 1, clientMutationId: 'q' })).rejects.toMatchObject({ failure: { field: 'caller' } })
  })

  it('does nothing without the desktop host', async () => {
    vi.mocked(isTauri).mockReturnValue(false)
    const client = createDesktopClient()
    expect(client.onOutbox(() => undefined)).toBeTypeOf('function')
    expect(client.onSentCopiesChanged(() => undefined)).toBeTypeOf('function')
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

  it('does not begin an already cancelled verification', async () => {
    const controller = new AbortController()
    controller.abort()
    const client = createDesktopClient()
    await expect(client.verifyAccountConnection('account', 2, controller.signal)).rejects.toMatchObject({ name: 'AbortError' })
    expect(invoke).not.toHaveBeenCalled()
  })
})

describe('desktop provider help', () => {
  it('forwards only the provider ID to the fixed-help command', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: null, requestId: 'help' })
    const client = createDesktopClient()
    const providers = ['qq', '139', 'sina', 'feishu', 'lark']
    for (const providerId of providers) await client.openProviderHelp(providerId)
    expect(vi.mocked(invoke).mock.calls).toEqual(providers.map((providerId) => [
      'accounts_open_provider_help', { providerId },
    ]))
  })

  it.each(['fastmail', 'feishu-intl', 'lark-cn', 'https://help.sina.com.cn/comquestiondetail/view/1566/'])(
    'does not fall back to a URL opener when the host rejects %s', async (providerId) => {
      const open = vi.spyOn(window, 'open').mockReturnValue(null)
      try {
        vi.mocked(invoke).mockResolvedValue({ ok: false, error: { code: 'validation', message: '', field: 'providerId' }, requestId: 'help' })
        await expect(createDesktopClient().openProviderHelp(providerId)).rejects.toMatchObject({ failure: { code: 'validation', field: 'providerId' } })
        expect(vi.mocked(invoke).mock.calls).toEqual([['accounts_open_provider_help', { providerId }]])
        expect(open).not.toHaveBeenCalled()
      } finally { open.mockRestore() }
    },
  )
})

describe('unsaved connection verification boundary', () => {
  const connection: ConnectionInput = {
    emailAddress: 'synthetic@163.com', providerId: 'netease163',
    imap: { host: 'imap.163.com', port: 993, tls: 'implicit', username: 'synthetic@163.com' },
    smtp: { host: 'smtp.163.com', port: 465, tls: 'implicit', username: 'synthetic@163.com' },
    authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-authorization-code' } },
    route: { kind: 'custom', proxyKind: 'socks5', host: 'proxy.example.com', port: 1080, dns: 'proxy', imageProxyDns: false, confirmImageProxyDns: false,
      authentication: { username: 'proxy-user', password: { action: 'replace', value: 'synthetic-proxy-secret' } } },
  }
  const protocols = {
    imap: { status: 'verified', stage: 'complete', code: null },
    smtp: { status: 'failed', stage: 'authentication', code: 'authentication' },
  }

  it('tests the complete transient connection and stops listening for cancellation after its bound report completes', async () => {
    vi.mocked(invoke).mockImplementation(async (_command, args) => {
      const { verificationId } = (args as { request: { verificationId: string } }).request
      return { ok: true, data: { verificationId, ...protocols }, requestId: 'test' }
    })
    const controller = new AbortController()
    const result = await createDesktopClient().verifyDraftConnection(connection, controller.signal)
    expect(result).toMatchObject(protocols)
    expect(result.verificationId).toMatch(/^[0-9a-f-]{36}$/)
    expect(invoke).toHaveBeenCalledExactlyOnceWith('accounts_verify_draft_connection', { request: { verificationId: result.verificationId, connection } })
    controller.abort()
    expect(invoke).toHaveBeenCalledOnce()
  })

  it('cancels one unique attempt without cancelling a concurrent test or accepting a late success', async () => {
    const complete = new Map<string, (value: unknown) => void>()
    vi.mocked(invoke).mockImplementation((command, args) => {
      if (command !== 'accounts_verify_draft_connection') return Promise.resolve({ ok: true, data: null, requestId: 'cancel' })
      const { verificationId } = (args as { request: { verificationId: string } }).request
      return new Promise((resolve) => { complete.set(verificationId, resolve) })
    })
    const client = createDesktopClient()
    const controller = new AbortController()
    const first = client.verifyDraftConnection(connection, controller.signal).catch((error: unknown) => error)
    const second = client.verifyDraftConnection(connection)
    const [firstId, secondId] = [...complete.keys()]
    expect(firstId).not.toBe(secondId)
    controller.abort()
    expect(invoke).toHaveBeenLastCalledWith('accounts_cancel_draft_verification', { request: { verificationId: firstId } })
    complete.get(secondId)!({ ok: true, data: { verificationId: secondId, ...protocols }, requestId: 'second' })
    await expect(second).resolves.toMatchObject({ verificationId: secondId, ...protocols })
    complete.get(firstId)!({ ok: true, data: { verificationId: firstId, ...protocols }, requestId: 'first' })
    expect(await first).toMatchObject({ name: 'AbortError' })
    expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual([
      'accounts_verify_draft_connection', 'accounts_verify_draft_connection', 'accounts_cancel_draft_verification',
    ])
  })

  it('refuses a report from a different attempt', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: { verificationId: 'different-attempt', ...protocols }, requestId: 'wrong' })
    await expect(createDesktopClient().verifyDraftConnection(connection)).rejects.toBeInstanceOf(MailxClientError)
  })

  it('does not start a test which was already cancelled', async () => {
    const controller = new AbortController()
    controller.abort()
    await expect(createDesktopClient().verifyDraftConnection(connection, controller.signal)).rejects.toMatchObject({ name: 'AbortError' })
    expect(invoke).not.toHaveBeenCalled()
  })
})

describe('desktop assistant commands', () => {
  it('sends only the named arguments and unwraps the results', async () => {
    const consent = { connectionId: 'c', baseUrl: 'https://api.example.test/v1' }
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: consent, requestId: 'request' })
    const client = createDesktopClient()
    await client.assistantStart({ kind: { type: 'chat', message: 'hi' } })
    await client.assistantCancel('run-1')
    await client.assistantSnapshot('run-1')
    await client.assistantRespond('run-1', 'req-1', true)
    await expect(client.assistantGrantConsent()).resolves.toEqual(consent)
    await client.assistantRevokeConsent()
    await client.assistantListConversations({ limit: 20, cursor: 'c|1' })
    await client.assistantListConversations()
    await client.assistantGetConversation('conv-1')
    await client.assistantDeleteConversation('conv-1')
    await client.assistantClearConversations()
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ['assistant_start', { request: { kind: { type: 'chat', message: 'hi' } } }],
      ['assistant_cancel', { runId: 'run-1' }],
      ['assistant_snapshot', { runId: 'run-1' }],
      ['assistant_respond', { runId: 'run-1', requestId: 'req-1', approved: true }],
      ['assistant_grant_consent', undefined],
      ['assistant_revoke_consent', undefined],
      ['assistant_list_conversations', { limit: 20, cursor: 'c|1' }],
      ['assistant_list_conversations', { limit: undefined, cursor: undefined }],
      ['assistant_get_conversation', { conversationId: 'conv-1' }],
      ['assistant_delete_conversation', { conversationId: 'conv-1' }],
      ['assistant_clear_conversations', undefined],
    ])
  })

  it('maps a refused answer to its stable code', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: false, error: { code: 'conflict', message: 'x', field: null }, requestId: 'request' })
    await expect(createDesktopClient().assistantRespond('run', 'old', false)).rejects.toMatchObject({ failure: { code: 'conflict' } })
  })
})
