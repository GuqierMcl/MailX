import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { AccountSyncState, EmailDetail, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { Workbench } from './workbench'

const account = exampleAccount()
const inbox: Folder = {
  id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox',
  selectable: true, unreadCount: 0, totalCount: 0,
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
    coverage: { headersComplete, bodiesComplete: false, syncedAfter: '2026-09-18', syncedBefore: null },
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
  client.listEmails.mockResolvedValue(page(items))
  client.getEmail.mockImplementation(async (id) => detail(items.find((item) => item.id === id)!))
  let listener: (state: AccountSyncState) => void = () => undefined
  client.onSyncState.mockImplementation((next) => {
    listener = next
    return () => undefined
  })
  const idle = () => act(() => listener({ accountId: account.id, phase: 'idle', lastSyncedAt: '2026-10-18T08:00:00.000Z', failure: null }))
  return { client, idle }
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
    expect(client.listFolders).toHaveBeenCalledTimes(2)
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
    client.listEmails.mockResolvedValueOnce(page([email('initial')], 'old-cursor'))
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
    client.listEmails.mockReturnValueOnce(pendingAll.promise)
    render(<Workbench client={client} />)
    await screen.findByRole('button', { name: '收件箱' })
    client.listEmails.mockResolvedValueOnce(page([email('unread')]))
    await userEvent.click(screen.getByRole('tab', { name: '未读' }))
    await screen.findByRole('list', { name: '邮件列表' })
    await act(async () => pendingAll.resolve(page([email('old', { isRead: true })])))
    expect(rows()).toHaveLength(1)
    expect(rows()[0]).toHaveTextContent('邮件 unread')
    expect(client.listEmails).toHaveBeenLastCalledWith(expect.objectContaining({ unread: true, cursor: null, limit: 100 }))
  })

  it('keeps existing rows and retries the same cursor after a pagination failure', async () => {
    const { client } = setup([email('first')])
    client.listEmails.mockResolvedValueOnce(page([email('first')], 'retry-cursor'))
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

  it('distinguishes an unsynced inbox, an unsupported folder and a completed empty inbox', async () => {
    const { client, idle } = setup()
    client.listEmails.mockResolvedValue(page([], null, false))
    render(<Workbench client={client} />)
    expect(await screen.findByText('正在同步邮件…')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '已发送' }))
    expect(await screen.findByText('当前版本只同步收件箱')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '收件箱' }))
    expect(await screen.findByText('正在同步邮件…')).toBeInTheDocument()
    client.listEmails.mockResolvedValue(page())
    idle()
    expect(await screen.findByText('这里还没有邮件')).toBeInTheDocument()
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
})
