import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { AccountSyncState, EmailDetail, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Workbench } from './workbench'

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
  return { summary, to: [], cc: [], replyTo: [], body: null, attachments: [] }
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

describe('synced inbox headers', () => {
  it('requests 100 headers, appends the cursor page and hides load more at the end', async () => {
    const items = Array.from({ length: 103 }, (_, index) => email(String(index)))
    const { client } = setup(items)
    client.listEmails.mockImplementation(async (query) => query.cursor ? page(items.slice(100)) : page(items.slice(0, 100), 'page-two'))
    render(<Workbench client={client} />)
    await screen.findByRole('list', { name: '邮件列表' })
    expect(rows()).toHaveLength(100)
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ limit: 100, cursor: null, folderRole: 'inbox' }))
    await userEvent.click(screen.getByRole('button', { name: '加载更多' }))
    await waitFor(() => expect(rows()).toHaveLength(103))
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ limit: 100, cursor: 'page-two', folderRole: 'inbox' }))
    expect(new Set(rows().map((row) => row.textContent)).size).toBe(103)
    expect(screen.queryByRole('button', { name: '加载更多' })).not.toBeInTheDocument()
  })

  it('reloads folders once for a burst of folder changes, and not at all when revisions stay put', async () => {
    const { client, emit } = setup([email('one')])
    render(<Workbench client={client} />)
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

  it('refreshes the first page and folders on idle without clearing the selected header or flashing skeletons', async () => {
    const selected = email('selected')
    const { client, idle } = setup([selected])
    render(<Workbench client={client} />)
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
    expect(rows()[1]).toHaveAttribute('aria-current', 'true')
    expect(within(rows()[1]).queryByLabelText('未读')).not.toBeInTheDocument()
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: null, limit: 100 }))
    // Folder reloads are coalesced, so they land shortly after the event.
    await waitFor(() => expect(client.listFolders).toHaveBeenCalledTimes(2), { timeout: 3000 })
    expect(client.getEmail).toHaveBeenCalledTimes(1)
    expect(screen.getByText('正文尚未下载')).toBeInTheDocument()
  })

  it('clears a selection missing from the refreshed first page and ignores its pending detail', async () => {
    const selected = email('removed')
    const { client, idle } = setup([selected])
    const pendingDetail = deferred<EmailDetail>()
    client.getEmail.mockReturnValue(pendingDetail.promise)
    render(<Workbench client={client} />)
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    client.listEmails.mockResolvedValue(page([email('remaining')]))
    idle()
    await waitFor(() => expect(rows()[0]).toHaveTextContent('邮件 remaining'))
    expect(rows()[0]).not.toHaveAttribute('aria-current')
    expect(screen.queryByRole('status', { name: '正在加载邮件' })).not.toBeInTheDocument()
    await act(async () => pendingDetail.resolve(detail(selected)))
    expect(screen.queryByRole('heading', { name: selected.subject })).not.toBeInTheDocument()
    expect(screen.getAllByText('未选择邮件').length).toBeGreaterThan(0)
  })

  it('ignores a pagination response that arrives after a sync refresh', async () => {
    const { client, idle } = setup([email('initial')])
    client.listEmails.mockResolvedValue(page([email('initial')], 'old-cursor'))
    const pendingPage = deferred<EmailPage>()
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
    await screen.findByText('这里还没有邮件')
    const pending = deferred<EmailPage>()
    client.listEmails.mockReturnValueOnce(pending.promise)
    idle()
    expect(screen.getByText('这里还没有邮件')).toBeInTheDocument()
    expect(screen.queryByRole('status', { name: '正在加载邮件' })).not.toBeInTheDocument()
    await act(async () => pending.resolve(page()))
  })

  it('shows a missing subject, omits a missing preview and keeps the header-only body notice', async () => {
    const { client } = setup([email('empty-fields', { subject: '', preview: '' })])
    render(<Workbench client={client} />)
    await screen.findByRole('list', { name: '邮件列表' })
    expect(within(rows()[0]).getByText('（无主题）')).toBeInTheDocument()
    expect(rows()[0].querySelector('p')).toBeNull()
    await userEvent.click(rows()[0])
    expect(await screen.findByRole('heading', { name: '（无主题）' })).toBeInTheDocument()
    expect(screen.getByText('正文尚未下载')).toBeInTheDocument()
    expect(screen.queryByRole('region', { name: '附件' })).not.toBeInTheDocument()
  })

  it('starts a search with the first-page cursor and preserves the page limit', async () => {
    const { client } = setup([email('first')])
    client.listEmails.mockResolvedValue(page([email('first')], 'unused-cursor'))
    render(<Workbench client={client} />)
    await screen.findByRole('list', { name: '邮件列表' })
    client.listEmails.mockResolvedValueOnce(page())
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索邮件' }), { target: { value: '  收件人  ' } })
    await screen.findByText('没有匹配的邮件')
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ text: '收件人', cursor: null, limit: 100 }))
  })

  it('keeps search, extra pages and selection when an unrelated folder finishes', async () => {
    const items = Array.from({ length: 103 }, (_, index) => email(String(index)))
    const { client, emit } = setup(items)
    client.listEmails.mockImplementation(async (query) => query.cursor ? page(items.slice(100)) : page(items.slice(0, 100), 'page-two'))
    render(<Workbench client={client} />)
    await screen.findByRole('list', { name: '邮件列表' })
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索邮件' }), { target: { value: '邮件' } })
    await waitFor(() => expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ text: '邮件' })))
    await userEvent.click(screen.getByRole('button', { name: '加载更多' }))
    await waitFor(() => expect(rows()).toHaveLength(103))
    await userEvent.click(rows()[102])
    await screen.findByRole('heading', { name: items[102].subject })
    const calls = client.listEmails.mock.calls.length
    emit(exampleSyncState({ sequence: 2, folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent', { dataRevision: 1 })] }))
    await waitFor(() => expect(client.listFolders).toHaveBeenCalledTimes(2), { timeout: 3000 })
    expect(client.listEmails).toHaveBeenCalledTimes(calls)
    expect(rows()).toHaveLength(103)
    expect(rows()[102]).toHaveAttribute('aria-current', 'true')
    expect(screen.getByRole('searchbox', { name: '搜索邮件' })).toHaveValue('邮件')
  })

  it('shows committed folder results before the account finishes and refreshes all loaded pages', async () => {
    let items = Array.from({ length: 103 }, (_, index) => email(String(index)))
    const { client, emit } = setup(items)
    client.listEmails.mockImplementation(async (query) => query.cursor ? page(items.slice(100)) : page(items.slice(0, 100), 'page-two'))
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
    await screen.findByRole('list', { name: '邮件列表' })
    expect(screen.getByText(/同步覆盖受限/)).toBeInTheDocument()
    expect(screen.queryByText('正在同步邮件…')).not.toBeInTheDocument()
    emit(exampleSyncState({ sequence: 2, folders: [exampleFolderSyncState('inbox', { phase: 'failed', failure: { code: 'protocol', message: '目录暂时不可用。', detail: null, willRetry: true } })] }))
    expect(rows()[0]).toHaveTextContent('邮件 cached')
    expect(screen.getByRole('status', { name: '邮件同步状态' })).toHaveTextContent('邮件同步失败 目录暂时不可用。')
    await userEvent.click(screen.getByRole('tab', { name: '未读' }))
    client.listEmails.mockResolvedValue(page([], null, false))
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索邮件' }), { target: { value: '没有' } })
    expect(await screen.findByText('没有匹配的邮件')).toBeInTheDocument()
    expect(screen.getByText(/邮件同步失败/)).toBeInTheDocument()
  })

  it('does not let a late state query overwrite a newer event', async () => {
    const { client, emit } = setup()
    const snapshot = deferred<AccountSyncState[]>()
    client.listSyncStates.mockReturnValue(snapshot.promise)
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
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
    render(<Workbench client={client} />)
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
    expect(rows()).toHaveLength(1)
    expect(rows()[0]).toHaveTextContent('邮件 replacement')
    expect(screen.queryByText(selected.subject)).not.toBeInTheDocument()
  })

  it('leaves a folder that becomes a Noselect container and hides its old selected mail', async () => {
    const sentMail = email('sent-only', { folderId: 'sent' })
    const { client, emit } = setup([sentMail])
    client.listEmails.mockImplementation(async (query) => page(query.folderId === 'sent' ? [sentMail] : []))
    render(<Workbench client={client} />)
    await screen.findByRole('button', { name: '已发送' })
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { name: sentMail.subject })
    client.listFolders.mockResolvedValue([inbox, { ...inbox, id: 'sent', name: 'Sent', role: 'sent', selectable: false }])
    emit(exampleSyncState({ sequence: 2, folderRevision: 1, folders: [exampleFolderSyncState('inbox')] }))
    await screen.findByRole('heading', { name: '全部收件箱' })
    expect(screen.queryByText(sentMail.subject)).not.toBeInTheDocument()
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ folderRole: 'inbox', folderId: null }))
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    expect(screen.getByRole('heading', { name: '全部收件箱' })).toBeInTheDocument()
  })
})
