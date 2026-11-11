import { fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { EmailDetail, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

const account = exampleAccount({ displayName: '工作邮箱' })
const folder = (id: string, role: Folder['role']): Folder => ({ id, accountId: account.id, parentId: null, name: id, role, selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0, archivedCount: 0, archivedUnreadCount: 0 })
const inbox = folder('inbox', 'inbox')
const sent = folder('sent', 'sent')

function email(id: string, from: { name: string | null; address: string }, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    senderGroup: null,
    id, accountId: account.id, folderId: inbox.id, subject: `主题 ${id}`, from, preview: `预览 ${id}`,
    receivedAt: '2026-10-18T08:00:00.000Z', isRead: true, isStarred: false,
    hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

const alice = { name: 'Alice', address: 'alice@example.com' }
const bob = { name: 'Bob', address: 'bob@example.com' }
// Newest first, as the host lists them.
const items = [
  email('a3', alice, { isRead: false }),
  email('b1', bob),
  email('a2', alice, { isRead: false }),
  email('a1', alice),
]

function page(rows: EmailSummary[]): EmailPage {
  return { items: rows, nextCursor: null, coverage: { headersComplete: true, bodiesComplete: false, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }
}

/** What the host does for `groupSenders`: Alice's mail collapses into her newest message. */
function listRows(query: { groupSenders: boolean; fromAddress: string | null }) {
  const rows = items.filter((row) => !query.fromAddress || row.from.address === query.fromAddress)
  if (!query.groupSenders) return rows
  const mine = rows.filter((row) => row.from.address === alice.address)
  return rows.filter((row) => row.from.address !== alice.address || row.id === mine[0].id).map((row) =>
    row.from.address === alice.address
      ? { ...row, senderGroup: { address: alice.address, total: mine.length, unread: mine.filter((m) => !m.isRead).length } }
      : row)
}

async function setup() {
  const client = testClient([account])
  client.listFolders.mockResolvedValue([inbox, sent])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState(inbox.id), exampleFolderSyncState(sent.id)] })])
  client.listEmails.mockImplementation(async (query) => page(listRows(query)))
  client.selectEmails.mockResolvedValue({
    items: [items[1]].map((row) => ({ id: row.id, accountId: account.id, folderId: inbox.id, isRead: true, isStarred: false, archived: false, localOnly: false })),
    truncated: false,
  })
  client.getEmail.mockImplementation(async (id): Promise<EmailDetail> =>
    ({ summary: items.find((row) => row.id === id)!, to: [], cc: [], replyTo: [], body: null, bodyFailure: null, attachments: [] }))
  render(<>{withTitleBar(<Workbench client={client} />)}<Toaster /></>)
  await screen.findByRole('list', { name: '邮件列表' })
  await waitFor(() => expect(rows()).toHaveLength(2))
  return { client }
}

const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
const groupRow = () => rows().find((row) => row.hasAttribute('data-sender-group'))!
const bobRow = () => rows().find((row) => within(row).queryByText('主题 b1'))!
const lastQuery = (client: Awaited<ReturnType<typeof setup>>['client']) => client.listEmails.mock.calls.at(-1)![0]

afterEach(() => vi.restoreAllMocks())

describe('grouped sender rows', () => {
  it('collapses a grouped sender into one row with counts, only in inbox views', async () => {
    const { client } = await setup()
    expect(lastQuery(client)).toMatchObject({ folderRole: 'inbox', groupSenders: true })
    const row = groupRow()
    expect(row).toBeDefined()
    expect(within(row).getByRole('button', { name: 'Alice，3 封邮件，2 封未读' })).toBeInTheDocument()
    expect(within(row).getByText('Alice')).toBeInTheDocument()
    expect(within(row).getByText('3 封')).toBeInTheDocument()
    expect(within(row).getByText('2 封未读')).toBeInTheDocument()
    expect(within(row).getByText('主题 a3')).toBeInTheDocument()
    expect(within(row).getByText('预览 a3')).toBeInTheDocument()
    expect(row.className).toBe(bobRow().className)

    const nav = within(screen.getByRole('navigation', { name: '邮箱导航' }))
    await userEvent.click(nav.getByRole('button', { name: '已发送' }))
    await waitFor(() => expect(lastQuery(client)).toMatchObject({ folderRole: 'sent', groupSenders: false }))
    await userEvent.click(nav.getByRole('button', { name: '星标' }))
    await waitFor(() => expect(lastQuery(client)).toMatchObject({ starred: true, groupSenders: false }))
  })

  it('opens the contact view on click and Enter, never the reader', async () => {
    const { client } = await setup()
    await userEvent.click(groupRow())
    await screen.findByRole('button', { name: '返回全部收件箱' })
    await waitFor(() => expect(rows()).toHaveLength(3))
    expect(lastQuery(client)).toMatchObject({ fromAddress: 'alice@example.com', groupSenders: false })
    expect(screen.getByRole('heading', { level: 1, name: 'Alice' })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { level: 2, name: '主题 a3' })).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: '返回全部收件箱' }))
    await waitFor(() => expect(rows()).toHaveLength(2))
    within(groupRow()).getByRole('button').focus()
    await userEvent.keyboard('{Enter}')
    await screen.findByRole('button', { name: '返回全部收件箱' })
    expect(screen.queryByRole('heading', { level: 2, name: '主题 a3' })).not.toBeInTheDocument()
  })

  it('is never shown as the opened message and has no quick actions', async () => {
    await setup()
    await userEvent.click(bobRow())
    await screen.findByRole('heading', { level: 2, name: '主题 b1' })
    await userEvent.hover(groupRow())
    expect(groupRow().querySelector('[data-row-action]')).toBeNull()
    expect(within(groupRow()).getAllByRole('button')).toHaveLength(1)
    expect(groupRow()).not.toHaveAttribute('aria-current')
    expect(bobRow()).toHaveAttribute('aria-current', 'true')
  })

  it('offers only the contact items in its right-click menu', async () => {
    await setup()
    fireEvent.contextMenu(groupRow())
    const menu = await screen.findByRole('menu', { name: '邮件菜单' })
    expect(within(menu).getAllByRole('menuitem').map((item) => item.textContent)).toEqual(['查看此联系人的邮件', '取消聚合此联系人'])
  })

  it('stops grouping from the menu', async () => {
    const { client } = await setup()
    fireEvent.contextMenu(groupRow())
    const menu = await screen.findByRole('menu', { name: '邮件菜单' })
    await userEvent.click(within(menu).getByRole('menuitem', { name: '取消聚合此联系人' }))
    expect(client.setContactAggregation).toHaveBeenCalledWith('alice@example.com', false)
  })

  it('cannot be picked, and select all and ranges leave it out', async () => {
    const { client } = await setup()
    await userEvent.click(screen.getByRole('button', { name: '选择' }))
    expect(within(groupRow()).queryByRole('checkbox')).not.toBeInTheDocument()
    expect(groupRow()).not.toHaveAttribute('data-picked')
    // A click on it neither picks nor opens the contact view.
    await userEvent.click(groupRow())
    expect(screen.queryByRole('button', { name: '返回全部收件箱' })).not.toBeInTheDocument()
    expect(screen.getByText('点选邮件')).toBeInTheDocument()

    // Shift-range from Bob across the group row picks only Bob.
    await userEvent.click(bobRow())
    await userEvent.keyboard('{Shift>}')
    await userEvent.click(groupRow())
    await userEvent.keyboard('{/Shift}')
    const toolbar = within(screen.getByRole('toolbar', { name: '所选邮件的操作' }))
    expect(toolbar.getByText('已选 1 封')).toBeInTheDocument()

    await userEvent.click(toolbar.getByRole('button', { name: '全选' }))
    await waitFor(() => expect(client.selectEmails).toHaveBeenCalled())
    await waitFor(() => expect(toolbar.getByRole('button', { name: '取消全选' })).toBeInTheDocument())
    expect(toolbar.getByText('已选 1 封')).toBeInTheDocument()
  })

  it('does not start picking on Cmd+click', async () => {
    await setup()
    await userEvent.keyboard('{Meta>}')
    await userEvent.click(groupRow())
    await userEvent.keyboard('{/Meta}')
    expect(screen.queryByRole('toolbar', { name: '所选邮件的操作' })).not.toBeInTheDocument()
  })
})
