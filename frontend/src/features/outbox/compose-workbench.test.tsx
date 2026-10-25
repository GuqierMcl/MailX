import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Toaster } from '@/components/ui/toast'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Workbench } from '@/features/mail/workbench'
import type { Draft, DraftChanged, DraftSummary, EmailDetail, EmailSummary, Folder, OutboxChanged, OutboxItem } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { withTitleBar } from '@/test/title-bar'

const account = exampleAccount()
const inbox: Folder = { id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox', selectable: true, countsKnown: true, unreadCount: 0, totalCount: 1 }
const mail: EmailSummary = {
  id: 'mail-1', accountId: account.id, folderId: 'inbox', subject: '季度计划', from: { name: '陈晓', address: 'chen@example.com' }, preview: '',
  receivedAt: '2026-10-18T08:00:00.000Z', isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', pendingChanges: false,
}
const detail: EmailDetail = { summary: mail, to: [], cc: [], replyTo: [], body: { text: '正文', hasHtml: false, truncated: false, remoteImagesBlocked: 0 }, bodyFailure: null, attachments: [] }

function draft(id = '3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b'): Draft {
  return { id, accountId: account.id, kind: 'new', sourceEmailId: null, to: [{ name: null, address: 'li@example.org' }], cc: [], bcc: [], subject: '草稿主题', bodyHtml: '<p>草稿正文</p>', bodyText: '草稿正文', inReplyTo: null, references: null, createdAt: '2026-10-23T00:00:00Z', updatedAt: '2026-10-23T00:00:00Z', revision: 2 }
}
const summary = (d: Draft): DraftSummary => ({ id: d.id, accountId: d.accountId, kind: d.kind, sourceEmailId: null, to: d.to, subject: d.subject, preview: d.bodyText, updatedAt: d.updatedAt, revision: d.revision })

function item(overrides: Partial<OutboxItem> = {}): OutboxItem {
  return {
    id: 'out-1', draftId: 'draft-1', accountId: account.id, messageId: '<m@x>', subject: '外发邮件', recipients: ['li@example.org'], recipientCount: 1,
    state: 'queued', notBefore: '2026-10-23T12:00:10.000Z', dataStartedAt: null, attempts: 0, nextAttemptAt: null, firstQueuedAt: '2026-10-23T12:00:00.000Z',
    error: null, acceptedAt: null, serverReply: null, serverCopyMessageId: null, resolution: null, supersededBy: null, createdAt: '2026-10-23T12:00:00.000Z', updatedAt: '2026-10-23T12:00:00.000Z', ...overrides,
  }
}

function setup(options: { drafts?: Draft[]; outbox?: OutboxItem[] } = {}) {
  const client = testClient([account])
  client.listFolders.mockResolvedValue([inbox])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState('inbox')] })])
  client.listEmails.mockResolvedValue({ items: [mail], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, syncedAfter: null, syncedBefore: null, windowCapped: false } })
  client.getEmail.mockResolvedValue(detail)
  const drafts = options.drafts ?? []
  const outbox = { current: options.outbox ?? [] }
  client.listDrafts.mockImplementation(async () => drafts.map(summary))
  client.getDraft.mockImplementation(async (id) => drafts.find((d) => d.id === id) ?? draft(id))
  client.listOutbox.mockImplementation(async () => structuredClone(outbox.current))
  client.createDraft.mockResolvedValue(draft('11111111-1111-4111-8111-111111111111'))
  client.createDraftFrom.mockResolvedValue(draft('22222222-2222-4222-8222-222222222222'))
  const draftListeners = new Set<(change: DraftChanged) => void>()
  client.onDrafts.mockImplementation((listener) => { draftListeners.add(listener); return () => { draftListeners.delete(listener) } })
  const listeners: {
    outbox: (change: OutboxChanged) => void
    open: (request: { accountId: string; emailId: string | null; outboxId?: string | null }) => void
  } = { outbox: () => undefined, open: () => undefined }
  client.onOutbox.mockImplementation((listener) => { listeners.outbox = listener; return () => undefined })
  client.onNotificationOpen.mockImplementation((listener) => { listeners.open = listener; return () => undefined })
  render(withTitleBar(<><Toaster /><Workbench client={client} /></>))
  return { client, listeners, outbox, drafts, emitDraft: (change: DraftChanged) => draftListeners.forEach((listener) => listener(change)) }
}

/** The island's rows; navigation items are list items too. */
const rows = (label: string) => within(screen.getByRole('list', { name: label })).getAllByRole('listitem')
const row = async (label: string) => (await screen.findByRole('list', { name: label }) && rows(label)[0])
afterEach(() => vi.restoreAllMocks())

describe('compose entry points', () => {
  it('"写邮件" in the navigation creates a draft for the context account and opens its window', async () => {
    const { client } = setup()
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(screen.getByRole('button', { name: '写邮件' }))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith('11111111-1111-4111-8111-111111111111', true))
    // The unified inbox is not one account, so the host's default applies.
    expect(client.createDraft).toHaveBeenCalledWith(expect.objectContaining({ accountId: null, to: [], subject: '', bodyHtml: '' }))
  })

  it('uses the account of the folder being viewed', async () => {
    const { client } = setup()
    await userEvent.click(await screen.findByRole('button', { name: '收件箱' }))
    await userEvent.click(screen.getByRole('button', { name: '写邮件' }))
    await waitFor(() => expect(client.createDraft).toHaveBeenCalledWith(expect.objectContaining({ accountId: account.id })))
  })

  it('Cmd+N does the same', async () => {
    const { client } = setup()
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.keyboard('{Meta>}n{/Meta}')
    await waitFor(() => expect(client.createDraft).toHaveBeenCalledTimes(1))
    await userEvent.keyboard('{Control>}n{/Control}')
    await waitFor(() => expect(client.createDraft).toHaveBeenCalledTimes(2))
  })

  it('the command palette offers 写邮件', async () => {
    const { client } = setup()
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await userEvent.click(await screen.findByRole('option', { name: '写邮件' }))
    await waitFor(() => expect(client.createDraft).toHaveBeenCalled())
    await waitFor(() => expect(client.openComposer).toHaveBeenCalled())
  })

  it.each([['回复', 'reply'], ['全部回复', 'replyAll'], ['转发', 'forward']] as const)('the reader button %s creates a %s draft from the open message', async (label, kind) => {
    const { client } = setup()
    await userEvent.click(await row('邮件列表'))
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    await userEvent.click(screen.getByRole('button', { name: label }))
    await waitFor(() => expect(client.createDraftFrom).toHaveBeenCalledWith({ emailId: 'mail-1', kind, clientMutationId: expect.any(String) }))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith('22222222-2222-4222-8222-222222222222', true))
  })

  it('fetches the body and retries once when the core says bodyNotCached', async () => {
    const { client } = setup()
    client.createDraftFrom.mockRejectedValueOnce(new MailxClientError({ code: 'bodyNotCached', message: 'x', field: null }))
    client.fetchBody.mockResolvedValue(detail)
    await userEvent.click(await row('邮件列表'))
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    await userEvent.click(screen.getByRole('button', { name: '回复' }))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith('22222222-2222-4222-8222-222222222222', true))
    expect(client.fetchBody).toHaveBeenCalledWith('mail-1')
    expect(client.createDraftFrom).toHaveBeenCalledTimes(2)
    // The retry reuses the mutation id, so a landed first attempt cannot make two drafts.
    expect(client.createDraftFrom.mock.calls[0][0].clientMutationId).toBe(client.createDraftFrom.mock.calls[1][0].clientMutationId)
  })

  it('says so when the body cannot be fetched', async () => {
    const { client } = setup()
    client.createDraftFrom.mockRejectedValue(new MailxClientError({ code: 'bodyNotCached', message: 'x', field: null }))
    client.fetchBody.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', message: '网络错误', field: null }))
    await userEvent.click(await row('邮件列表'))
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    await userEvent.click(screen.getByRole('button', { name: '回复' }))
    expect((await screen.findAllByText('无法打开写信窗口')).length).toBeGreaterThan(0)
    expect(client.openComposer).not.toHaveBeenCalled()
  })
})

describe('drafts and outbox navigation', () => {
  it('always lists 草稿 with a count, and hides 发件箱 while nothing is unfinished', async () => {
    setup({ drafts: [draft()] })
    await screen.findByRole('list', { name: '邮件列表' })
    const drafts = screen.getByRole('button', { name: '草稿' })
    expect(drafts).toBeInTheDocument()
    await waitFor(() => expect(within(drafts.closest('li')!).getByLabelText('1 份草稿')).toBeInTheDocument())
    expect(screen.queryByRole('button', { name: '发件箱' })).not.toBeInTheDocument()
  })

  it('shows 发件箱 with its count once something is queued, sending, failed or unknown', async () => {
    setup({ outbox: [item(), item({ id: 'out-2', draftId: 'draft-2', state: 'failed' })] })
    const outbox = await screen.findByRole('button', { name: '发件箱' })
    expect(within(outbox.closest('li')!).getByLabelText('2 封未完成的邮件')).toHaveTextContent('2')
  })

  it('lists drafts in the island and shows a preview with 继续编辑 and 删除', async () => {
    const d = draft()
    const { client } = setup({ drafts: [d] })
    await userEvent.click((await screen.findAllByRole('button', { name: '草稿' }))[0])
    const draftRow = await row('草稿列表')
    expect(draftRow).toHaveTextContent('草稿主题')
    await userEvent.click(draftRow)
    expect(await screen.findByRole('heading', { level: 2, name: '草稿主题' })).toBeInTheDocument()
    expect(screen.getByRole('article')).toHaveTextContent('草稿正文')
    const edit = screen.getByRole('button', { name: '继续编辑' })
    for (let i = 0; i < 40 && document.activeElement !== edit; i++) await userEvent.tab()
    expect(await screen.findByText('继续编辑')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '继续编辑' }))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith(d.id, false))

    await userEvent.click(screen.getByRole('button', { name: '删除' }))
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '删除' }))
    await waitFor(() => expect(client.deleteDraft).toHaveBeenCalledWith(d.id))
  })

  it('clears the selection instead of showing 已不存在 when the selected draft is deleted', async () => {
    const d = draft()
    const { drafts, emitDraft } = setup({ drafts: [d] })
    await userEvent.click((await screen.findAllByRole('button', { name: '草稿' }))[0])
    await userEvent.click(await row('草稿列表'))
    await screen.findByRole('heading', { level: 2, name: '草稿主题' })
    drafts.splice(0)
    act(() => emitDraft({ draftId: d.id, accountId: account.id, change: 'deleted' }))
    await waitFor(() => expect(screen.queryByRole('heading', { level: 2, name: '草稿主题' })).not.toBeInTheDocument())
    expect(screen.queryByText('这份草稿已不存在')).not.toBeInTheDocument()
  })
})

async function openOutbox(entry: OutboxItem) {
  const ctx = setup({ outbox: [entry] })
  await userEvent.click(await screen.findByRole('button', { name: '发件箱' }))
  await userEvent.click(await row('发件箱列表'))
  await screen.findByRole('heading', { level: 2, name: entry.subject })
  return ctx
}

describe('outbox items', () => {
  it('queued: 撤销 cancels and reopens the composer; 立即重试 only when a retry is scheduled', async () => {
    const { client } = await openOutbox(item())
    client.cancelOutbox.mockResolvedValue(item({ state: 'canceled' }))
    expect(screen.queryByRole('button', { name: '立即重试' })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '撤销' }))
    await waitFor(() => expect(client.cancelOutbox).toHaveBeenCalledWith('out-1'))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith('draft-1'))
  })

  it('queued with a scheduled retry offers 立即重试', async () => {
    const { client } = await openOutbox(item({ attempts: 1, nextAttemptAt: '2026-10-23T12:00:30.000Z', error: { reason: 'connect', smtpCode: null, text: null, rejected: [] } }))
    client.retryOutboxNow.mockResolvedValue(item())
    expect(screen.getByText(/无法连接发信服务器/)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '立即重试' }))
    await waitFor(() => expect(client.retryOutboxNow).toHaveBeenCalledWith('out-1'))
  })

  it('failed: shows rejected recipients and the Chinese reason, and 编辑并重新发送 opens the draft', async () => {
    const { client } = await openOutbox(item({
      state: 'failed', attempts: 1,
      error: { reason: 'recipientsRejected', smtpCode: 550, text: null, rejected: [{ address: 'bad@example.org', code: 550, text: 'no such user' }] },
    }))
    expect(screen.getByText('服务器拒绝了部分收件人，邮件没有发出')).toBeInTheDocument()
    const list = screen.getByRole('list', { name: '被拒绝的收件人' })
    expect(list).toHaveTextContent('bad@example.org')
    expect(list).toHaveTextContent('no such user')
    await userEvent.click(screen.getByRole('button', { name: '编辑并重新发送' }))
    expect(client.openComposer).toHaveBeenCalledWith('draft-1', false)
  })

  it('unknown: explains, marks as sent directly, and confirms before a resend', async () => {
    const { client } = await openOutbox(item({ state: 'unknown', error: { reason: 'unknown', smtpCode: null, text: 'connectionLost', rejected: [] } }))
    client.resolveUnknown.mockResolvedValue(item({ state: 'accepted' }))
    expect(screen.getByText('服务器可能已收到这封邮件')).toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: '确认未发出，重新发送' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(client.resolveUnknown).not.toHaveBeenCalled()
    await userEvent.click(within(dialog).getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())

    await userEvent.click(screen.getByRole('button', { name: '标记为已发出' }))
    await waitFor(() => expect(client.resolveUnknown).toHaveBeenCalledWith({ id: 'out-1', resolution: 'markSent', clientMutationId: expect.any(String) }))

    await userEvent.click(screen.getByRole('button', { name: '确认未发出，重新发送' }))
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '确认未发出，重新发送' }))
    await waitFor(() => expect(client.resolveUnknown).toHaveBeenLastCalledWith({ id: 'out-1', resolution: 'resend', clientMutationId: expect.any(String) }))
  })

  it('hides a failed item whose draft was sent again', async () => {
    setup({ outbox: [
      item({ id: 'old', state: 'failed', createdAt: '2026-10-23T11:00:00.000Z' }),
      item({ id: 'new', state: 'sending', createdAt: '2026-10-23T12:00:00.000Z' }),
    ] })
    await userEvent.click(await screen.findByRole('button', { name: '发件箱' }))
    await screen.findByRole('list', { name: '发件箱列表' })
    expect(rows('发件箱列表')).toHaveLength(1)
  })

  it('falls back to the inbox when the last unfinished item completes', async () => {
    const { listeners, outbox } = await openOutbox(item())
    outbox.current = []
    await act(async () => listeners.outbox({ accountId: account.id, outboxId: 'out-1', draftId: 'draft-1', state: 'accepted', attempts: 1, nextAttemptAt: null, errorReason: null }))
    expect(await screen.findByRole('heading', { name: '全部收件箱' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '发件箱' })).not.toBeInTheDocument()
  })
})

describe('send toasts and notifications', () => {
  it('queued with an undo window: "将在 N 秒后发送" with 撤销, which cancels and reopens the composer', async () => {
    const { client, listeners } = setup()
    await screen.findByRole('list', { name: '邮件列表' })
    const now = Date.now()
    client.getOutbox.mockResolvedValue(item({ firstQueuedAt: new Date(now).toISOString(), notBefore: new Date(now + 10_000).toISOString() }))
    client.cancelOutbox.mockResolvedValue(item({ state: 'canceled' }))
    await act(async () => listeners.outbox({ accountId: account.id, outboxId: 'out-1', draftId: 'draft-1', state: 'queued', attempts: 0, nextAttemptAt: null, errorReason: null }))
    expect((await screen.findAllByText('将在 10 秒后发送')).length).toBeGreaterThan(0)
    await userEvent.click((await screen.findAllByRole('button', { name: '撤销', hidden: true }))[0])
    await waitFor(() => expect(client.cancelOutbox).toHaveBeenCalledWith('out-1'))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith('draft-1'))
  })

  it('says it cannot undo once sending has started', async () => {
    const { client, listeners } = setup()
    await screen.findByRole('list', { name: '邮件列表' })
    const now = Date.now()
    client.getOutbox.mockResolvedValue(item({ firstQueuedAt: new Date(now).toISOString(), notBefore: new Date(now + 10_000).toISOString() }))
    client.cancelOutbox.mockRejectedValue(new MailxClientError({ code: 'alreadySending', message: 'x', field: null }))
    await act(async () => listeners.outbox({ accountId: account.id, outboxId: 'out-1', draftId: 'draft-1', state: 'queued', attempts: 0, nextAttemptAt: null, errorReason: null }))
    await userEvent.click((await screen.findAllByRole('button', { name: '撤销', hidden: true }))[0])
    expect((await screen.findAllByText('已开始发送，无法撤销')).length).toBeGreaterThan(0)
    expect(client.openComposer).not.toHaveBeenCalled()
  })

  it('announces sending, sent, failed and unknown; failures link to the outbox item', async () => {
    const { listeners, outbox } = setup({ outbox: [item({ state: 'failed' })] })
    await screen.findByRole('list', { name: '邮件列表' })
    const emit = (state: OutboxChanged['state'], errorReason: string | null = null) =>
      act(async () => listeners.outbox({ accountId: account.id, outboxId: 'out-1', draftId: 'draft-1', state, attempts: 1, nextAttemptAt: null, errorReason }))
    await emit('sending')
    expect((await screen.findAllByText('正在发送…')).length).toBeGreaterThan(0)
    await emit('accepted')
    expect((await screen.findAllByText('已发送')).length).toBeGreaterThan(0)
    await emit('failed', 'authRejected')
    expect((await screen.findAllByText(/发送失败：发信服务器拒绝了账号登录/)).length).toBeGreaterThan(0)
    await userEvent.click((await screen.findAllByRole('button', { name: '查看', hidden: true }))[0])
    expect(await screen.findByRole('heading', { name: '发件箱' })).toBeInTheDocument()
    expect(outbox.current).toHaveLength(1)
    await emit('unknown')
    expect((await screen.findAllByText('可能已发送，需要确认')).length).toBeGreaterThan(0)
  })

  it('selects the outbox item named by a clicked notification', async () => {
    const { listeners } = setup({ outbox: [item({ id: 'out-9', subject: '通知里的邮件', state: 'failed' })] })
    await screen.findByRole('list', { name: '邮件列表' })
    await act(async () => listeners.open({ accountId: account.id, emailId: null, outboxId: 'out-9' }))
    expect(await screen.findByRole('heading', { level: 2, name: '通知里的邮件' })).toBeInTheDocument()
    expect(rows('发件箱列表')[0]).toHaveAttribute('aria-current', 'true')
  })
})
