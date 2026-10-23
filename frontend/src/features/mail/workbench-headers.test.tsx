import { act, cleanup, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { AccountSyncState, EmailDetail, EmailPage, EmailSummary, FlagWriteBack, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { MailxClientError } from '@/lib/mailx-client'
import { searchFor } from '@/test/search'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

const DEFAULT_SETTINGS = { appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask' as const, senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS }
const account = exampleAccount()
const inbox: Folder = {
  id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox',
  selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0,
}

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id, accountId: account.id, folderId: inbox.id, subject: `邮件 ${id}`,
    from: { name: '发件人', address: 'sender@example.com' }, preview: '',
    receivedAt: '2026-10-18T08:00:00.000Z', isRead: false, isStarred: false,
    hasAttachment: false, bodyState: 'missing', pendingChanges: false, ...overrides,
  }
}

function detail(summary: EmailSummary): EmailDetail {
  return { summary, to: [], cc: [], replyTo: [], body: null, bodyFailure: null, attachments: [] }
}

function page(items: EmailSummary[] = [], nextCursor: string | null = null, headersComplete = true): EmailPage {
  return {
    items, nextCursor,
    coverage: { headersComplete, bodiesComplete: false, syncedAfter: '2026-09-18', syncedBefore: null, windowCapped: false },
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((complete) => { resolve = complete })
  return { promise, resolve }
}

function setup(items: EmailSummary[] = []) {
  const client = testClient([account])
  client.listFolders.mockResolvedValue([inbox, { ...inbox, id: 'sent', name: 'Sent', role: 'sent' }])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent')] })])
  client.listEmails.mockResolvedValue(page(items))
  client.getEmail.mockImplementation(async (id) => detail(items.find((item) => item.id === id)!))
  let listener: (state: AccountSyncState) => void = () => undefined
  client.onSyncState.mockImplementation((next) => {
    listener = next
    return () => undefined
  })
  let revision = 1
  const emit = (state: AccountSyncState) => act(() => listener(state))
  const idle = () => emit(exampleSyncState({ sequence: ++revision, folders: [exampleFolderSyncState('inbox', { dataRevision: revision }), exampleFolderSyncState('sent')] }))
  return { client, idle, emit }
}

const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
/** Like `rows`, but an empty list (which renders an empty state instead of a list) counts as no rows. */
const listedRows = () => screen.queryAllByRole('listitem').filter((row) => row.closest('[aria-label="邮件列表"]'))

describe('synced inbox headers', () => {
  it('requests 100 headers, appends the cursor page and hides load more at the end', async () => {
    const items = Array.from({ length: 103 }, (_, index) => email(String(index)))
    const { client } = setup(items)
    client.listEmails.mockImplementation(async (query) => query.cursor ? page(items.slice(100)) : page(items.slice(0, 100), 'page-two'))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    expect(rows()).toHaveLength(100)
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ limit: 100, cursor: null, folderRole: 'inbox' }))
    await userEvent.click(screen.getByRole('button', { name: '加载更多' }))
    await waitFor(() => expect(rows()).toHaveLength(103))
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ limit: 100, cursor: 'page-two', folderRole: 'inbox' }))
    expect(new Set(rows().map((row) => row.textContent)).size).toBe(103)
    expect(screen.queryByRole('button', { name: '加载更多' })).not.toBeInTheDocument()
  })

  it('retains both loaded pages and the selection when a body completes and refreshes previews', async () => {
    const first = email('first')
    const last = email('last')
    const { client } = setup([first, last])
    let ready = false
    client.listEmails.mockImplementation(async (query) => query.cursor
      ? page([{ ...last, preview: ready ? 'new preview' : '', bodyState: ready ? 'ready' : 'missing' }])
      : page([first], 'second-page'))
    const pending = deferred<EmailDetail>()
    client.fetchBody.mockReturnValue(pending.promise)
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(screen.getByRole('button', { name: '加载更多' }))
    await waitFor(() => expect(rows()).toHaveLength(2))
    await userEvent.click(rows()[1])
    await screen.findByRole('status', { name: '正在加载正文' })
    ready = true
    await act(async () => pending.resolve({ ...detail({ ...last, preview: 'new preview', bodyState: 'ready' }),
      body: { text: 'downloaded body', hasHtml: false, truncated: false, remoteImagesBlocked: 0 } }))
    expect(await screen.findByText('downloaded body')).toBeInTheDocument()
    await waitFor(() => expect(rows()[1]).toHaveTextContent('new preview'))
    expect(rows()).toHaveLength(2)
    expect(rows()[1]).toHaveAttribute('aria-current', 'true')
    expect(client.fetchBody).toHaveBeenCalledTimes(1)
    expect(client.listEmails.mock.calls.slice(-2).map(([query]) => query.cursor)).toEqual([null, 'second-page'])
  })

  it('reloads folders once for a burst of folder changes, and not at all when revisions stay put', async () => {
    const { client, emit } = setup([email('one')])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await waitFor(() => expect(client.listFolders).toHaveBeenCalledTimes(1))

    // A round that changed nothing repeats the same revisions.
    emit(exampleSyncState({ sequence: 2, folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent')] }))
    await new Promise((resolve) => setTimeout(resolve, 400))
    expect(client.listFolders).toHaveBeenCalledTimes(1)

    for (const [sequence, revision] of [[3, 2], [4, 3], [5, 4]]) {
      emit(exampleSyncState({ sequence, folders: [exampleFolderSyncState('inbox', { dataRevision: revision }), exampleFolderSyncState('sent', { dataRevision: revision })] }))
    }
    await waitFor(() => expect(client.listFolders).toHaveBeenCalledTimes(2), { timeout: 3000 })
    await new Promise((resolve) => setTimeout(resolve, 400))
    expect(client.listFolders).toHaveBeenCalledTimes(2)
  })

  it('refetches a cached list that changed while another mailbox was shown', async () => {
    const { client, idle } = setup([email('before')])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByText('邮件 before')
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    await screen.findByRole('heading', { name: '已发送' })

    // New inbox mail arrives while Sent is on screen.
    client.listEmails.mockResolvedValue(page([email('after'), email('before')]))
    idle()
    await userEvent.click(screen.getByRole('button', { name: '全部收件箱' }))
    expect(await screen.findByText('邮件 after')).toBeInTheDocument()
  })

  it('serves an unchanged mailbox from cache when switching back', async () => {
    const { client } = setup([email('cached')])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByText('邮件 cached')
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    await screen.findByRole('heading', { name: '已发送' })
    const calls = client.listEmails.mock.calls.length
    await userEvent.click(screen.getByRole('button', { name: '全部收件箱' }))
    expect(screen.getByText('邮件 cached')).toBeInTheDocument()
    expect(client.listEmails).toHaveBeenCalledTimes(calls)
  })

  it('refreshes the first page and folders on idle without clearing the selected header or flashing skeletons', async () => {
    const selected = email('selected')
    const { client, idle } = setup([selected])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: selected.subject })
    const refreshed = deferred<EmailPage>()
    client.listEmails.mockReturnValueOnce(refreshed.promise)
    idle()
    expect(rows()[0]).toHaveAttribute('aria-current', 'true')
    expect(screen.getByRole('heading', { name: selected.subject })).toBeInTheDocument()
    expect(screen.queryByRole('status', { name: '正在加载邮件' })).not.toBeInTheDocument()
    await act(async () => refreshed.resolve(page([email('new'), { ...selected, isRead: true }])))
    // Query results reach components on the next notification tick.
    await waitFor(() => expect(rows()[1]).toHaveAttribute('aria-current', 'true'))
    expect(within(rows()[1]).queryByLabelText('未读')).not.toBeInTheDocument()
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: null, limit: 100 }))
    // Folder reloads are coalesced, so they land shortly after the event.
    await waitFor(() => expect(client.listFolders).toHaveBeenCalledTimes(2), { timeout: 3000 })
    expect(client.getEmail).toHaveBeenCalledTimes(1)
    expect(await screen.findByText('正文获取失败')).toBeInTheDocument()
  })

  it('keeps the opened message when a refreshed list no longer contains it, and only drops the highlight', async () => {
    const opened = email('removed')
    const { client, idle } = setup([opened])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: opened.subject })
    client.listEmails.mockResolvedValue(page([email('remaining')]))
    idle()
    await waitFor(() => expect(rows()[0]).toHaveTextContent('邮件 remaining'))
    // The list moved on; the reader did not.
    expect(rows()[0]).not.toHaveAttribute('aria-current')
    expect(screen.getByRole('heading', { name: opened.subject })).toBeInTheDocument()
    expect(screen.queryByText('未选择邮件')).not.toBeInTheDocument()
  })

  it('ignores a pagination response that arrives after a sync refresh', async () => {
    const { client, idle } = setup([email('initial')])
    client.listEmails.mockResolvedValue(page([email('initial')], 'old-cursor'))
    const pendingPage = deferred<EmailPage>()
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    client.listEmails.mockReturnValueOnce(pendingPage.promise)
    await userEvent.click(screen.getByRole('button', { name: '加载更多' }))
    client.listEmails.mockResolvedValueOnce(page([email('fresh')]))
    idle()
    await waitFor(() => expect(rows()[0]).toHaveTextContent('邮件 fresh'))
    await act(async () => pendingPage.resolve(page([email('stale')], 'stale-cursor')))
    expect(rows()).toHaveLength(1)
    expect(screen.queryByText('邮件 stale')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '加载更多' })).not.toBeInTheDocument()
  })

  it('ignores an older query response after changing to unread', async () => {
    const { client } = setup()
    const pendingAll = deferred<EmailPage>()
    client.listEmails.mockImplementation((query) => query.unread ? Promise.resolve(page([email('unread')])) : pendingAll.promise)
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('button', { name: '收件箱' })
    await userEvent.click(screen.getByRole('tab', { name: '未读' }))
    await screen.findByRole('list', { name: '邮件列表' })
    await act(async () => pendingAll.resolve(page([email('old', { isRead: true })])))
    expect(rows()).toHaveLength(1)
    expect(rows()[0]).toHaveTextContent('邮件 unread')
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ unread: true, cursor: null, limit: 100 }))
  })

  it('keeps existing rows and retries the same cursor after a pagination failure', async () => {
    const { client } = setup([email('first')])
    client.listEmails.mockResolvedValue(page([email('first')], 'retry-cursor'))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    client.listEmails.mockRejectedValueOnce(new Error('private transport detail'))
    await userEvent.click(screen.getByRole('button', { name: '加载更多' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('与桌面应用的通信中断')
    expect(rows()).toHaveLength(1)
    client.listEmails.mockResolvedValueOnce(page([email('second')]))
    await userEvent.click(screen.getByRole('button', { name: '加载更多' }))
    await waitFor(() => expect(rows()).toHaveLength(2))
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: 'retry-cursor' }))
  })

  it('distinguishes waiting, syncing, failed and completed empty folders without inferring progress from coverage', async () => {
    const { client, emit } = setup()
    client.listEmails.mockResolvedValue(page([], null, false))
    client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState('inbox', { phase: 'syncing' }), exampleFolderSyncState('sent', { phase: 'waiting' })] })])
    render(withTitleBar(<Workbench client={client} />))
    expect(await screen.findByText('正在同步邮件…')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    expect(await screen.findByText('等待同步邮件…')).toBeInTheDocument()
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ folderId: 'sent', folderRole: null }))
    emit(exampleSyncState({ sequence: 2, folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent', { phase: 'failed', failure: { code: 'protocol', message: '此文件夹暂时不可用。', detail: null, willRetry: true } })] }))
    expect(await screen.findByText('邮件同步失败')).toBeInTheDocument()
    expect(screen.getByText('此文件夹暂时不可用。')).toBeInTheDocument()
    emit(exampleSyncState({ sequence: 3, folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent')] }))
    expect(await screen.findByText('这里还没有邮件')).toBeInTheDocument()
    // Incomplete coverage alone cannot label an idle folder as still syncing.
    expect(screen.queryByText('正在同步邮件…')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '全部收件箱' }))
    expect(await screen.findByText('这里还没有邮件')).toBeInTheDocument()
  })

  it('does not show a loading skeleton when an empty completed inbox refreshes', async () => {
    const { client, idle } = setup()
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByText('这里还没有邮件')
    const pending = deferred<EmailPage>()
    client.listEmails.mockReturnValueOnce(pending.promise)
    idle()
    expect(screen.getByText('这里还没有邮件')).toBeInTheDocument()
    expect(screen.queryByRole('status', { name: '正在加载邮件' })).not.toBeInTheDocument()
    await act(async () => pending.resolve(page()))
  })

  it('shows a missing subject, omits a missing preview and shows a controlled body download failure', async () => {
    const { client } = setup([email('empty-fields', { subject: '', preview: '' })])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    expect(within(rows()[0]).getByText('（无主题）')).toBeInTheDocument()
    expect(rows()[0].querySelector('p')).toBeNull()
    await userEvent.click(rows()[0])
    expect(await screen.findByRole('heading', { name: '（无主题）' })).toBeInTheDocument()
    expect(await screen.findByText('正文获取失败')).toBeInTheDocument()
    expect(screen.queryByRole('region', { name: '附件' })).not.toBeInTheDocument()
  })

  it('starts a search with the first-page cursor and preserves the page limit', async () => {
    const { client } = setup([email('first')])
    client.listEmails.mockResolvedValue(page([email('first')], 'unused-cursor'))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    client.listEmails.mockResolvedValueOnce(page())
    await searchFor('  收件人  ')
    await screen.findByText('没有匹配的邮件')
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ text: '收件人', cursor: null, limit: 100 }))
  })

  it('keeps search, extra pages and selection when an unrelated folder finishes', async () => {
    const items = Array.from({ length: 103 }, (_, index) => email(String(index)))
    const { client, emit } = setup(items)
    client.listEmails.mockImplementation(async (query) => query.cursor ? page(items.slice(100)) : page(items.slice(0, 100), 'page-two'))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await searchFor('邮件')
    await waitFor(() => expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ text: '邮件' })))
    // The search starts a new query; its first page renders a notification tick after the call.
    await userEvent.click(await screen.findByRole('button', { name: '加载更多' }))
    await waitFor(() => expect(rows()).toHaveLength(103))
    await userEvent.click(rows()[102])
    await screen.findByRole('heading', { name: items[102].subject })
    const calls = client.listEmails.mock.calls.length
    emit(exampleSyncState({ sequence: 2, folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent', { dataRevision: 1 })] }))
    await waitFor(() => expect(client.listFolders).toHaveBeenCalledTimes(2), { timeout: 3000 })
    expect(client.listEmails).toHaveBeenCalledTimes(calls)
    expect(rows()).toHaveLength(103)
    expect(rows()[102]).toHaveAttribute('aria-current', 'true')
    expect(screen.getByRole('button', { name: '搜索邮件' })).toHaveTextContent('邮件')
  })

  it('shows committed folder results before the account finishes and refreshes all loaded pages', async () => {
    let items = Array.from({ length: 103 }, (_, index) => email(String(index)))
    const { client, emit } = setup(items)
    client.listEmails.mockImplementation(async (query) => query.cursor ? page(items.slice(100)) : page(items.slice(0, 100), 'page-two'))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(screen.getByRole('button', { name: '加载更多' }))
    await waitFor(() => expect(rows()).toHaveLength(103))
    await userEvent.click(rows()[102])
    await screen.findByRole('heading', { name: items[102].subject })
    items = [email('new'), ...items.map((item) => ({ ...item, isRead: true }))]
    emit(exampleSyncState({ phase: 'syncing', sequence: 2, folders: [exampleFolderSyncState('inbox', { dataRevision: 1 }), exampleFolderSyncState('sent', { phase: 'syncing' })] }))
    await waitFor(() => expect(rows()).toHaveLength(104))
    expect(rows()[0]).toHaveTextContent('邮件 new')
    expect(rows()[103]).toHaveAttribute('aria-current', 'true')
    expect(within(rows()[103]).queryByLabelText('未读')).not.toBeInTheDocument()
    expect(screen.getByRole('heading', { name: '邮件 102' })).toBeInTheDocument()
    expect(screen.queryByText('正在同步邮件…')).not.toBeInTheDocument()
  })

  it('rejects stale source and task events without replacing cache or the current status', async () => {
    const { client, emit } = setup([email('cached')])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    emit(exampleSyncState({ generation: 2, sequence: 1, folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent')] }))
    const calls = client.listEmails.mock.calls.length
    for (const stale of [
      exampleSyncState({ mailboxSourceId: 'old-source', generation: 10, sequence: 10 }),
      exampleSyncState({ generation: 1, sequence: 100 }),
      exampleSyncState({ generation: 2, sequence: 0 }),
    ]) {
      emit({ ...stale, folders: [exampleFolderSyncState('inbox', { phase: 'failed', dataRevision: 99 })] })
    }
    expect(client.listEmails).toHaveBeenCalledTimes(calls)
    expect(rows()[0]).toHaveTextContent('邮件 cached')
    expect(screen.queryByText('邮件同步失败')).not.toBeInTheDocument()
  })

  it('keeps cached rows on sync failure and distinguishes a capped idle window from syncing', async () => {
    const { client, emit } = setup([email('cached')])
    client.listEmails.mockResolvedValue({ ...page([email('cached')], null, false), coverage: { ...page().coverage, headersComplete: false, windowCapped: true } })
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    expect(screen.getByText(/同步覆盖受限/)).toBeInTheDocument()
    expect(screen.queryByText('正在同步邮件…')).not.toBeInTheDocument()
    emit(exampleSyncState({ sequence: 2, folders: [exampleFolderSyncState('inbox', { phase: 'failed', failure: { code: 'protocol', message: '目录暂时不可用。', detail: null, willRetry: true } })] }))
    expect(rows()[0]).toHaveTextContent('邮件 cached')
    // A failure no longer inserts a banner above the rows (it made the list jump);
    // the empty and filtered states below still explain it.
    expect(screen.queryByRole('status', { name: '邮件同步状态' })).not.toBeInTheDocument()
    expect(screen.queryByText(/邮件同步失败/)).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('tab', { name: '未读' }))
    client.listEmails.mockResolvedValue(page([], null, false))
    await searchFor('没有')
    expect(await screen.findByText('没有匹配的邮件')).toBeInTheDocument()
    expect(screen.getByText(/邮件同步失败/)).toBeInTheDocument()
  })

  it('does not let a late state query overwrite a newer event', async () => {
    const { client, emit } = setup()
    const snapshot = deferred<AccountSyncState[]>()
    client.listSyncStates.mockReturnValue(snapshot.promise)
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByText('等待同步邮件…')
    emit(exampleSyncState({ sequence: 5, folders: [exampleFolderSyncState('inbox', { phase: 'syncing' })] }))
    expect(await screen.findByText('正在同步邮件…')).toBeInTheDocument()
    await act(async () => snapshot.resolve([exampleSyncState({ sequence: 1, folders: [exampleFolderSyncState('inbox', { phase: 'failed' })] })]))
    expect(screen.getByText('正在同步邮件…')).toBeInTheDocument()
    expect(screen.queryByText('邮件同步失败')).not.toBeInTheDocument()
  })

  it('recovers missed directory notifications from the state query without repeated tree refreshes', async () => {
    const { client, emit } = setup()
    const oldFolders = deferred<Folder[]>()
    const sent: Folder = { ...inbox, id: 'sent', name: 'Sent', role: 'sent' }
    client.listFolders.mockReturnValueOnce(oldFolders.promise).mockResolvedValue([sent])
    const fresh = exampleSyncState({ sequence: 20, folderRevision: 1, folders: [exampleFolderSyncState('sent', { dataRevision: 2 })] })
    client.listSyncStates.mockResolvedValue([fresh])
    render(withTitleBar(<Workbench client={client} />))
    await waitFor(() => expect(client.listFolders).toHaveBeenCalledTimes(1))
    // The initial account identity is not known yet, so this event cannot be accepted.
    emit(fresh)
    await act(async () => oldFolders.resolve([]))
    expect(await screen.findByRole('button', { name: '已发送' })).toBeInTheDocument()
    expect(client.listFolders).toHaveBeenCalledTimes(2)
    emit({ ...fresh, sequence: 21 })
    expect(client.listFolders).toHaveBeenCalledTimes(2)
    expect(client.listSyncStates).toHaveBeenCalledTimes(2)
  })

  it('refreshes directory and message results when a new task reuses the same revision numbers', async () => {
    const { client, emit } = setup([email('before-restart')])
    const archive: Folder = { ...inbox, id: 'archive', name: 'Removed', role: 'other' }
    client.listFolders.mockResolvedValue([inbox, archive])
    client.listSyncStates.mockResolvedValue([exampleSyncState({
      generation: 1, sequence: 20, folderRevision: 1,
      folders: [exampleFolderSyncState('inbox', { dataRevision: 1 }), exampleFolderSyncState('archive', { dataRevision: 1 })],
    })])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('button', { name: 'Removed' })
    await screen.findByText('邮件 before-restart')
    await waitFor(() => expect(client.listFolders).toHaveBeenCalledTimes(2), { timeout: 3000 })
    client.listFolders.mockResolvedValue([inbox])
    client.listEmails.mockResolvedValue(page([email('after-restart')]))
    emit(exampleSyncState({
      generation: 2, sequence: 20, folderRevision: 1,
      folders: [exampleFolderSyncState('inbox', { dataRevision: 1 })],
    }))
    await screen.findByText('邮件 after-restart')
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Removed' })).not.toBeInTheDocument())
    expect(client.listFolders).toHaveBeenCalledTimes(3)
    expect(screen.queryByText('邮件 before-restart')).not.toBeInTheDocument()
  })

  it('hides old source rows and detail while the replacement query is still pending', async () => {
    const selected = email('old-source')
    const { client, emit } = setup([selected])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: selected.subject })
    const replacement = deferred<EmailPage>()
    client.listEmails.mockReturnValue(replacement.promise)
    client.listAccounts.mockResolvedValue([exampleAccount({ mailboxSourceId: 'new-source' })])
    emit(exampleSyncState({ sequence: 2, folderRevision: 1, folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent')] }))
    await waitFor(() => expect(screen.queryByText(selected.subject)).not.toBeInTheDocument())
    expect(screen.queryByRole('heading', { name: selected.subject })).not.toBeInTheDocument()
    emit(exampleSyncState({ generation: 99, sequence: 99, folders: [exampleFolderSyncState('inbox', { dataRevision: 99 })] }))
    await act(async () => replacement.resolve(page([email('replacement')])))
    await waitFor(() => expect(rows()).toHaveLength(1))
    expect(rows()[0]).toHaveTextContent('邮件 replacement')
    expect(screen.queryByText(selected.subject)).not.toBeInTheDocument()
  })

  it('leaves a folder that becomes a Noselect container, and keeps the message that was open in it', async () => {
    const sentMail = email('sent-only', { folderId: 'sent' })
    const { client, emit } = setup([sentMail])
    client.listEmails.mockImplementation(async (query) => page(query.folderId === 'sent' ? [sentMail] : []))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('button', { name: '已发送' })
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: sentMail.subject })
    client.listFolders.mockResolvedValue([inbox, { ...inbox, id: 'sent', name: 'Sent', role: 'sent', selectable: false }])
    emit(exampleSyncState({ sequence: 2, folderRevision: 1, folders: [exampleFolderSyncState('inbox')] }))
    await screen.findByRole('heading', { name: '全部收件箱' })
    // The list is back on the unified inbox and no longer lists the message...
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ folderRole: 'inbox', folderId: null }))
    expect(listedRows()).toHaveLength(0)
    // ...but the reader is not part of the list, so the message stays open.
    expect(screen.getByRole('heading', { name: sentMail.subject })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    expect(screen.getByRole('heading', { name: '全部收件箱' })).toBeInTheDocument()
  })

  it('keeps the open message when the user switches to another mailbox', async () => {
    const opened = email('keep-me')
    const { client } = setup([opened])
    client.listEmails.mockImplementation(async (query) => page(query.folderId === 'sent' ? [] : [opened]))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: opened.subject })
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    await screen.findByRole('heading', { name: '已发送' })
    expect(listedRows()).toHaveLength(0)
    expect(screen.getByRole('heading', { name: opened.subject })).toBeInTheDocument()
  })

  it('keeps a message open after reading it removes it from the unread filter', async () => {
    const unread = email('unread-one', { isRead: false })
    const { client } = setup([unread])
    // The list answers like the core: once a message is read it is not unread any more.
    let readIds = new Set<string>()
    let confirmWriteBack: (written: FlagWriteBack) => void = () => undefined
    client.onFlagWriteBack.mockImplementation((listener) => { confirmWriteBack = listener; return () => undefined })
    client.setFlag.mockImplementation(async (id) => { readIds = new Set([...readIds, id]); return 'queued' })
    client.listEmails.mockImplementation(async (query) => page(query.unread && readIds.has(unread.id) ? [] : [{ ...unread, isRead: readIds.has(unread.id) }]))
    client.getEmail.mockImplementation(async (id) => ({ ...detail(unread), summary: { ...unread, id, isRead: readIds.has(id) } }))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(screen.getByRole('tab', { name: '未读' }))
    await waitFor(() => expect(rows()).toHaveLength(1))
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: unread.subject })
    await waitFor(() => expect(client.setFlag).toHaveBeenCalledWith(unread.id, 'read', true))
    // Until the list is refreshed the row stays, now shown as read: nothing jumps.
    await waitFor(() => expect(within(rows()[0]).queryByLabelText('未读')).not.toBeInTheDocument())
    // The server confirms, the list is re-read and the message leaves the unread filter...
    act(() => confirmWriteBack({ accountId: unread.accountId, sent: 1, localOnlyStars: 0 }))
    await waitFor(() => expect(listedRows()).toHaveLength(0))
    // ...and the message is still open, and can be marked unread again.
    expect(screen.getByRole('heading', { name: unread.subject })).toBeInTheDocument()
    await userEvent.click(await screen.findByRole('button', { name: '标为未读' }))
    await waitFor(() => expect(client.setFlag).toHaveBeenLastCalledWith(unread.id, 'read', false))
    expect(screen.getByRole('heading', { name: unread.subject })).toBeInTheDocument()
  })

  it('closes the reader when the message turns out to be gone for good', async () => {
    const gone = email('gone')
    const { client } = setup([gone])
    // The core refuses a message that was deleted or replaced: its detail is "not found".
    client.getEmail.mockRejectedValue(new MailxClientError({ code: 'notFound', message: '内容不存在。', field: null }))
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await waitFor(() => expect(screen.getAllByText('未选择邮件').length).toBeGreaterThan(0))
    expect(screen.queryByRole('button', { name: '关闭邮件' })).not.toBeInTheDocument()
  })

  it('closes with the close button and with Esc, and a search box keeps Esc for itself', async () => {
    const opened = email('closable')
    const { client } = setup([opened])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await userEvent.click(await screen.findByRole('button', { name: '关闭邮件' }))
    expect(screen.getAllByText('未选择邮件').length).toBeGreaterThan(0)
    // With nothing open there is nothing to act on, so the toolbar is gone entirely.
    expect(screen.queryByRole('toolbar', { name: '邮件操作' })).not.toBeInTheDocument()

    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: opened.subject })
    expect(screen.getByRole('toolbar', { name: '邮件操作' })).toBeInTheDocument()
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.getAllByText('未选择邮件').length).toBeGreaterThan(0))

    // Esc inside the search palette dismisses the palette, not the message.
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: opened.subject })
    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await screen.findByRole('combobox')
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('combobox')).not.toBeInTheDocument())
    expect(screen.getByRole('heading', { name: opened.subject })).toBeInTheDocument()
  })
})

describe('clicked notifications', () => {
  function withNotifications(items: EmailSummary[]) {
    const context = setup(items)
    let open: (request: { accountId: string; emailId: string | null }) => void = () => undefined
    context.client.onNotificationOpen.mockImplementation((listener) => { open = listener; return () => undefined })
    return { ...context, click: (request: { accountId: string; emailId: string | null }) => act(() => open(request)) }
  }

  it('opens the one message a notification was about, even when the list shows another mailbox', async () => {
    const arrived = email('arrived', { subject: '新到的周报' })
    const { client, click } = withNotifications([arrived])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(screen.getByRole('button', { name: /已发送/ }))
    await screen.findByRole('heading', { name: '已发送' })

    click({ accountId: account.id, emailId: arrived.id })
    expect(await screen.findByRole('heading', { name: '新到的周报' })).toBeInTheDocument()
    // The list is left where the user put it.
    expect(screen.getByRole('heading', { name: '已发送' })).toBeInTheDocument()
  })

  it("shows the account's inbox for a notification about several messages", async () => {
    const { client, click } = withNotifications([email('one'), email('two')])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(screen.getByRole('button', { name: /已发送/ }))
    await screen.findByRole('heading', { name: '已发送' })

    click({ accountId: account.id, emailId: null })
    await waitFor(() => expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ folderId: inbox.id })))
  })

  it('ignores a notification of an account that no longer exists', async () => {
    const { client, click } = withNotifications([email('one')])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    const calls = client.getEmail.mock.calls.length
    click({ accountId: 'gone', emailId: 'one' })
    expect(client.getEmail.mock.calls.length).toBe(calls)
  })
})

describe('folding the reader away', () => {
  const toggle = () => screen.getByRole('button', { name: /^(收起|展开)邮件内容$/ })

  it('has a fold button whose label and state follow the reader', async () => {
    const { client } = setup([email('one')])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    expect(toggle()).toHaveAttribute('aria-label', '收起邮件内容')
    expect(toggle()).toHaveAttribute('aria-expanded', 'true')
    await userEvent.click(toggle())
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '展开邮件内容'))
    expect(toggle()).toHaveAttribute('aria-expanded', 'false')
    await userEvent.click(toggle())
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '收起邮件内容'))
  })

  it('opens the reader again when a message is clicked, and keeps it folded otherwise', async () => {
    const one = email('one')
    const { client } = setup([one])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(toggle())
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '展开邮件内容'))
    // Sync and list refreshes do not unfold it.
    await userEvent.click(screen.getByRole('tab', { name: '未读' }))
    expect(toggle()).toHaveAttribute('aria-label', '展开邮件内容')
    await userEvent.click(screen.getByRole('tab', { name: '全部' }))

    await userEvent.click(rows()[0])
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '收起邮件内容'))
    expect(await screen.findByRole('heading', { name: one.subject })).toBeInTheDocument()
  })

  it('does not close a message with Esc while the reader is out of sight', async () => {
    const one = email('one')
    const { client } = setup([one])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: one.subject })
    await userEvent.click(toggle())
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '展开邮件内容'))
    await userEvent.keyboard('{Escape}')
    await userEvent.click(toggle())
    // Still the same message when it is shown again.
    expect(await screen.findByRole('heading', { name: one.subject })).toBeInTheDocument()
  })

  it('unfolds the reader for a clicked notification about one message', async () => {
    const arrived = email('arrived', { subject: '新到的周报' })
    const { client } = setup([arrived])
    let open: (request: { accountId: string; emailId: string | null }) => void = () => undefined
    client.onNotificationOpen.mockImplementation((listener) => { open = listener; return () => undefined })
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(toggle())
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '展开邮件内容'))
    act(() => open({ accountId: account.id, emailId: arrived.id }))
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '收起邮件内容'))
    expect(await screen.findByRole('heading', { name: '新到的周报' })).toBeInTheDocument()
  })
})


describe('the folded reader survives a restart', () => {
  const toggle = () => screen.getByRole('button', { name: /^(收起|展开)邮件内容$/ })
  const savedLayout = (client: ReturnType<typeof setup>['client'], readerCollapsed: boolean) =>
    client.getSettings.mockResolvedValue({
      settings: { ...DEFAULT_SETTINGS, layout: { ...DEFAULT_LAYOUT, readerCollapsed } }, stored: true, recovered: false,
    })

  it('starts folded when it was left folded, and shown by default', async () => {
    const folded = setup([email('one')])
    savedLayout(folded.client, true)
    render(withTitleBar(<Workbench client={folded.client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    expect(toggle()).toHaveAttribute('aria-label', '展开邮件内容')
    cleanup()

    const shown = setup([email('one')])
    render(withTitleBar(<Workbench client={shown.client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    expect(toggle()).toHaveAttribute('aria-label', '收起邮件内容')
  })

  it('saves the choice once things settle, not on every toggle, and carries the other widths along', async () => {
    const { client } = setup([email('one')])
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    for (let index = 0; index < 4; index += 1) await userEvent.click(toggle())
    await userEvent.click(toggle())
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '展开邮件内容'))
    expect(client.updateSettings).not.toHaveBeenCalled()
    await waitFor(() => expect(client.updateSettings).toHaveBeenCalledOnce(), { timeout: 2000 })
    expect(client.updateSettings).toHaveBeenCalledWith({ layout: { ...DEFAULT_LAYOUT, readerCollapsed: true } })
  })

  it('opening a message unfolds the reader and saves that too', async () => {
    const one = email('one')
    const { client } = setup([one])
    savedLayout(client, true)
    render(withTitleBar(<Workbench client={client} />))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-label', '收起邮件内容'))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ layout: { ...DEFAULT_LAYOUT, readerCollapsed: false } }), { timeout: 2000 })
  })
})
