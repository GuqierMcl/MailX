import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { EmailDetail, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

const account = exampleAccount({ displayName: '工作邮箱' })
const inbox: Folder = { id: 'inbox', accountId: account.id, parentId: null, name: 'inbox', role: 'inbox', selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0, archivedCount: 0, archivedUnreadCount: 0 }

function email(id: string, from: { name: string | null; address: string }, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    senderGroup: null,
    id, accountId: account.id, folderId: inbox.id, subject: `主题 ${id}`, from, preview: '',
    receivedAt: '2026-10-18T08:00:00.000Z', isRead: true, isStarred: false,
    hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

const alice = { name: 'Alice', address: 'alice@example.com' }
const bob = { name: 'Bob', address: 'bob@example.com' }
const items = [email('a1', alice), email('b1', bob), email('a2', { name: 'Alice', address: 'Alice@Example.com' }), email('b2', bob)]

function page(rows: EmailSummary[]): EmailPage {
  return { items: rows, nextCursor: null, coverage: { headersComplete: true, bodiesComplete: false, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }
}

async function setup() {
  const client = testClient([account])
  client.listFolders.mockResolvedValue([inbox])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState(inbox.id)] })])
  client.listEmails.mockImplementation(async (query) => page(items.filter((row) =>
    !query.fromAddress || row.from.address.toLowerCase() === query.fromAddress.toLowerCase())))
  client.getEmail.mockImplementation(async (id): Promise<EmailDetail> =>
    ({ summary: items.find((row) => row.id === id)!, to: [], cc: [], replyTo: [], body: null, bodyFailure: null, attachments: [] }))
  render(<>{withTitleBar(<Workbench client={client} />)}<Toaster /></>)
  await screen.findByRole('list', { name: '邮件列表' })
  await waitFor(() => expect(rows()).toHaveLength(4))
  return { client }
}

const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
const rowFor = (subject: string) => rows().find((row) => within(row).queryByText(subject))!

async function openMenu(subject: string) {
  fireEvent.contextMenu(rowFor(subject))
  return screen.findByRole('menu', { name: '邮件菜单' })
}

async function enterContact(subject = '主题 a1') {
  const menu = await openMenu(subject)
  await userEvent.click(within(menu).getByRole('menuitem', { name: '查看此联系人的邮件' }))
  await screen.findByRole('button', { name: '返回全部收件箱' })
  await waitFor(() => expect(rows()).toHaveLength(2))
}

afterEach(() => vi.restoreAllMocks())

describe('the contact view', () => {
  it('lists only that sender, case-insensitively, under a header with a back button', async () => {
    const { client } = await setup()
    await enterContact()
    expect(rows().map((row) => row.textContent)).toEqual([expect.stringContaining('主题 a1'), expect.stringContaining('主题 a2')])
    expect(screen.getByRole('heading', { level: 1, name: 'Alice' })).toBeInTheDocument()
    expect(screen.getByText('alice@example.com')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '返回全部收件箱' })).toHaveTextContent('全部收件箱')
    expect(screen.getByRole('tab', { name: '未读' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '选择' })).toBeInTheDocument()
    const query = client.listEmails.mock.calls.at(-1)![0]
    expect(query).toMatchObject({
      fromAddress: 'alice@example.com', folderId: null, folderRole: null, excludeFolderRoles: ['junk', 'trash'],
      archive: 'include', accountIds: null, groupSenders: false,
    })
    // The sidebar still points at the mailbox underneath.
    expect(screen.getByRole('navigation', { name: '邮箱导航' })).toBeInTheDocument()
  })

  it('does not offer the contact it is showing, but still offers grouping', async () => {
    await setup()
    await enterContact()
    const menu = await openMenu('主题 a1')
    expect(within(menu).queryByRole('menuitem', { name: '查看此联系人的邮件' })).not.toBeInTheDocument()
    expect(within(menu).getByRole('menuitem', { name: '聚合此联系人的邮件' })).toBeInTheDocument()
  })

  it('keeps the filter and search inside the contact', async () => {
    const { client } = await setup()
    await enterContact()
    await userEvent.click(screen.getByRole('tab', { name: '未读' }))
    await waitFor(() => expect(client.listEmails.mock.calls.at(-1)![0]).toMatchObject({ fromAddress: 'alice@example.com', unread: true }))
    expect(screen.getByRole('button', { name: '搜索邮件' })).toHaveTextContent('Alice')
  })

  it('does not carry the previous rows over while the first page loads', async () => {
    const { client } = await setup()
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    const original = client.listEmails.getMockImplementation()!
    client.listEmails.mockImplementation(async (query) => {
      if (query.fromAddress) await gate
      return original(query)
    })
    const menu = await openMenu('主题 a1')
    await userEvent.click(within(menu).getByRole('menuitem', { name: '查看此联系人的邮件' }))
    await screen.findByRole('button', { name: '返回全部收件箱' })
    expect(screen.queryByText('主题 b1')).not.toBeInTheDocument()
    expect(screen.queryByText('主题 a1')).not.toBeInTheDocument()
    await act(async () => release())
    await waitFor(() => expect(rows()).toHaveLength(2))
  })

  it('goes back with the button, Escape and the mouse back button', async () => {
    await setup()
    for (const back of [
      () => userEvent.click(screen.getByRole('button', { name: '返回全部收件箱' })),
      () => userEvent.keyboard('{Escape}'),
      () => { fireEvent.mouseUp(document.body, { button: 3 }) },
    ]) {
      await enterContact()
      await back()
      await waitFor(() => expect(rows()).toHaveLength(4))
      expect(screen.getByRole('heading', { level: 1, name: '全部收件箱' })).toBeInTheDocument()
    }
  })

  it('leaves Escape to selection mode first, then to the open message, then goes back', async () => {
    await setup()
    await enterContact()
    await userEvent.click(screen.getByRole('button', { name: '选择' }))
    await userEvent.keyboard('{Escape}')
    expect(screen.queryByRole('toolbar', { name: '所选邮件的操作' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '返回全部收件箱' })).toBeInTheDocument()

    await userEvent.click(rowFor('主题 a1'))
    await screen.findByRole('heading', { level: 2, name: '主题 a1' })
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('heading', { level: 2, name: '主题 a1' })).not.toBeInTheDocument())
    expect(screen.getByRole('button', { name: '返回全部收件箱' })).toBeInTheDocument()
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('button', { name: '返回全部收件箱' })).not.toBeInTheDocument())
  })

  it('does not close the open message by entering or leaving', async () => {
    await setup()
    await userEvent.click(rowFor('主题 b1'))
    await screen.findByRole('heading', { level: 2, name: '主题 b1' })
    await enterContact()
    expect(screen.getByRole('heading', { level: 2, name: '主题 b1' })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '返回全部收件箱' }))
    await waitFor(() => expect(rows()).toHaveLength(4))
    expect(screen.getByRole('heading', { level: 2, name: '主题 b1' })).toBeInTheDocument()
  })

  it('ends when another mailbox is chosen', async () => {
    await setup()
    await enterContact()
    await userEvent.click(within(screen.getByRole('navigation', { name: '邮箱导航' })).getByRole('button', { name: '收件箱' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: '返回全部收件箱' })).not.toBeInTheDocument())
    await waitFor(() => expect(rows()).toHaveLength(4))
  })

  it('brings the list back to where it was scrolled', async () => {
    const scrollTo = vi.fn()
    Element.prototype.scrollTo = scrollTo as unknown as typeof Element.prototype.scrollTo
    await setup()
    const viewport = document.querySelector('[data-slot="scroll-area-viewport"]')!
    Object.defineProperty(viewport, 'scrollTop', { configurable: true, value: 240 })
    fireEvent.scroll(viewport)
    await enterContact()
    scrollTo.mockClear()
    await userEvent.click(screen.getByRole('button', { name: '返回全部收件箱' }))
    await waitFor(() => expect(rows()).toHaveLength(4))
    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith(expect.objectContaining({ top: 240 })))
    delete (Element.prototype as { scrollTo?: unknown }).scrollTo
  })
})

describe('the slide between a list and a contact', () => {
  const pane = () => document.querySelector('[data-slot="list-pane"]')!

  it('slides in from the right on the way in and from the left on the way out', async () => {
    await setup()
    expect(pane().className).not.toContain('animate-in')
    await enterContact()
    expect(pane()).toHaveClass('animate-in', 'fade-in-0', 'duration-200', 'slide-in-from-right-6')
    await userEvent.click(screen.getByRole('button', { name: '返回全部收件箱' }))
    await waitFor(() => expect(rows()).toHaveLength(4))
    expect(pane()).toHaveClass('animate-in', 'slide-in-from-left-6')
  })

  it('does not slide for an ordinary mailbox switch', async () => {
    await setup()
    await userEvent.click(within(screen.getByRole('navigation', { name: '邮箱导航' })).getByRole('button', { name: '收件箱' }))
    expect(pane().className).not.toContain('slide-in')
  })

  it('does not move when the system asks for reduced motion', async () => {
    vi.mocked(window.matchMedia).mockImplementation((query: string) => ({
      matches: query.includes('reduce'), media: query, onchange: null,
      addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
    }))
    await setup()
    await enterContact()
    expect(pane().className).not.toContain('animate-in')
    expect(pane().className).not.toContain('slide-in')
  })
})

describe('grouping a contact from the menu', () => {
  it('groups, refreshes, and can be undone from the notice', async () => {
    const { client } = await setup()
    const listed = client.listContactAggregations.mock.calls.length
    const menu = await openMenu('主题 b1')
    await userEvent.click(within(menu).getByRole('menuitem', { name: '聚合此联系人的邮件' }))
    expect(client.setContactAggregation).toHaveBeenCalledWith('bob@example.com', true)
    await screen.findByText('已聚合 Bob 的邮件')
    await waitFor(() => expect(client.listContactAggregations.mock.calls.length).toBeGreaterThan(listed))
    const lists = client.listEmails.mock.calls.length
    await userEvent.click(screen.getByRole('button', { name: '撤销' }))
    await waitFor(() => expect(client.setContactAggregation).toHaveBeenLastCalledWith('bob@example.com', false))
    await screen.findByText('已取消聚合 Bob')
    expect(lists).toBeGreaterThan(0)
  })

  it('offers to stop grouping a contact that has a rule, ignoring case', async () => {
    const { client } = await (async () => {
      const made = await setupWithRule()
      return made
    })()
    const menu = await openMenu('主题 a1')
    await userEvent.click(within(menu).getByRole('menuitem', { name: '取消聚合此联系人' }))
    expect(client.setContactAggregation).toHaveBeenCalledWith('alice@example.com', false)
    await screen.findByText('已取消聚合 Alice')
  })

  it('says so when it fails', async () => {
    const { client } = await setup()
    client.setContactAggregation.mockRejectedValueOnce(new Error('boom'))
    const menu = await openMenu('主题 b1')
    await userEvent.click(within(menu).getByRole('menuitem', { name: '聚合此联系人的邮件' }))
    expect((await screen.findAllByText('无法聚合此联系人的邮件')).length).toBeGreaterThan(0)
  })
})

async function setupWithRule() {
  const client = testClient([account])
  client.listFolders.mockResolvedValue([inbox])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState(inbox.id)] })])
  client.listEmails.mockResolvedValue(page(items))
  client.listContactAggregations.mockResolvedValue([
    { address: 'ALICE@example.com', name: 'Alice', messageCount: 2, latestReceivedAt: null, createdAt: '2026-10-18T08:00:00.000Z' },
  ])
  render(<>{withTitleBar(<Workbench client={client} />)}<Toaster /></>)
  await screen.findByRole('list', { name: '邮件列表' })
  await waitFor(() => expect(rows()).toHaveLength(4))
  return { client }
}
