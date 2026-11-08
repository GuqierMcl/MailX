import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { EmailDetail, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

const account = exampleAccount({ displayName: '工作邮箱' })
function folder(id: string, role: Folder['role']): Folder {
  return { id, accountId: account.id, parentId: null, name: id, role, selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0, archivedCount: 0, archivedUnreadCount: 0 }
}
const inbox = folder('inbox', 'inbox')
const trash = folder('trash', 'trash')

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id, accountId: account.id, folderId: inbox.id, subject: `邮件 ${id}`,
    from: { name: '发件人', address: 'sender@example.com' }, preview: '',
    receivedAt: '2026-10-18T08:00:00.000Z', isRead: true, isStarred: false,
    hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

function page(items: EmailSummary[]): EmailPage {
  return { items, nextCursor: null, coverage: { headersComplete: true, bodiesComplete: false, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }
}

function detail(summary: EmailSummary): EmailDetail {
  return { summary, to: [], cc: [], replyTo: [], body: null, bodyFailure: null, attachments: [] }
}

async function setup(items: EmailSummary[], folders: Folder[] = [inbox, trash]) {
  const client = testClient([account])
  client.listFolders.mockResolvedValue(folders)
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: folders.map((row) => exampleFolderSyncState(row.id)) })])
  client.listEmails.mockResolvedValue(page(items))
  client.getEmail.mockImplementation(async (id) => detail(items.find((item) => item.id === id)!))
  const user = userEvent.setup()
  render(<>{withTitleBar(<Workbench client={client} />)}<Toaster /></>)
  await screen.findByRole('list', { name: '邮件列表' })
  return { client, user }
}

const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
const toolbar = () => within(screen.getByRole('toolbar', { name: '所选邮件的操作' }))

describe('selecting messages in the list', () => {
  it('turns the header into the selection bar and back, with actions disabled until something is picked', async () => {
    const { user } = await setup([email('a'), email('b')])
    expect(screen.getByRole('tab', { name: '全部' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '选择' }))
    expect(screen.queryByRole('tab', { name: '全部' })).not.toBeInTheDocument()
    expect(toolbar().getByText('点选邮件')).toBeInTheDocument()
    for (const name of ['标为已读', '加星标', '归档', '删除']) {
      expect(toolbar().getByRole('button', { name })).toBeDisabled()
    }
    // A plain click now picks instead of opening.
    await user.click(rows()[0])
    expect(toolbar().getByText('已选 1 封')).toBeInTheDocument()
    expect(within(rows()[0]).getByRole('checkbox')).toHaveAttribute('aria-checked', 'true')
    expect(rows()[0]).toHaveAttribute('data-picked')
    expect(toolbar().getByRole('button', { name: '删除' })).toBeEnabled()
    expect(screen.getByText('已选择 1 封邮件')).toBeInTheDocument()
    await user.click(toolbar().getByRole('button', { name: '完成' }))
    expect(screen.getByRole('tab', { name: '全部' })).toBeInTheDocument()
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument()
  })

  it('starts with only the clicked row on Cmd+click, leaves on Esc and shows the opened message again', async () => {
    const { user } = await setup([email('a'), email('b'), email('c')])
    await user.click(rows()[0])
    expect(await screen.findByRole('heading', { name: '邮件 a' })).toBeInTheDocument()
    await user.keyboard('{Meta>}')
    await user.click(rows()[2])
    await user.keyboard('{/Meta}')
    expect(toolbar().getByText('已选 1 封')).toBeInTheDocument()
    expect(within(rows()[0]).getByRole('checkbox')).toHaveAttribute('aria-checked', 'false')
    expect(screen.queryByRole('heading', { name: '邮件 a' })).not.toBeInTheDocument()
    // Shift picks the range from the last clicked row.
    await user.keyboard('{Shift>}')
    await user.click(rows()[0])
    await user.keyboard('{/Shift}')
    expect(toolbar().getByText('已选 3 封')).toBeInTheDocument()
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('toolbar', { name: '所选邮件的操作' })).not.toBeInTheDocument()
    expect(await screen.findByRole('heading', { name: '邮件 a' })).toBeInTheDocument()
  })

  it('selects everything in scope through the host, including messages not loaded, and says when it stopped at 500', async () => {
    const { client, user } = await setup([email('a'), email('b')])
    client.selectEmails.mockResolvedValue({
      items: ['a', 'b', 'c'].map((id) => ({ id, accountId: account.id, folderId: inbox.id, isRead: true, isStarred: false, archived: false, localOnly: false })),
      truncated: true,
    })
    await user.click(screen.getByRole('button', { name: '选择' }))
    await user.click(toolbar().getByRole('button', { name: '全选' }))
    expect(await toolbar().findByText('已选 3 封')).toBeInTheDocument()
    expect(client.selectEmails).toHaveBeenCalledWith(expect.objectContaining({ folderRole: 'inbox', limit: null, cursor: null }))
    expect(screen.getByText('一次最多处理 500 封')).toBeInTheDocument()
    await user.click(toolbar().getByRole('button', { name: '取消全选' }))
    expect(toolbar().getByText('点选邮件')).toBeInTheDocument()
    expect(screen.queryByText('一次最多处理 500 封')).not.toBeInTheDocument()
  })

  it('offers "mark as read" while any picked message is unread, and changes them in one call', async () => {
    const { client, user } = await setup([email('a', { isRead: false }), email('b')])
    await user.click(screen.getByRole('button', { name: '选择' }))
    await user.click(rows()[1])
    expect(toolbar().getByRole('button', { name: '标为未读' })).toBeEnabled()
    await user.click(rows()[0])
    await user.click(toolbar().getByRole('button', { name: '标为已读' }))
    await waitFor(() => expect(client.setFlags).toHaveBeenCalledWith(expect.arrayContaining(['a', 'b']), 'read', true))
    expect(await screen.findByText('已将 2 封标为已读')).toBeInTheDocument()
    // Now all are read, so the button turns the other way; the selection stays.
    expect(toolbar().getByRole('button', { name: '标为未读' })).toBeInTheDocument()
    expect(toolbar().getByText('已选 2 封')).toBeInTheDocument()
  })

  it('deletes with one toast and undo, keeping what could not be deleted selected', async () => {
    const { client, user } = await setup([email('a'), email('b'), email('c')])
    client.deleteEmails.mockResolvedValue({ batchId: 'batch-9', undoUntil: '2099-01-01T00:00:00.000Z', permanent: false, skippedIds: ['c'] })
    await user.click(screen.getByRole('button', { name: '选择' }))
    for (const row of rows()) await user.click(row)
    client.listEmails.mockResolvedValue(page([email('c')]))
    await user.click(toolbar().getByRole('button', { name: '删除' }))
    await waitFor(() => expect(client.deleteEmails).toHaveBeenCalledWith(['a', 'b', 'c'], false, true))
    expect(await screen.findByText('2 封已删除，1 封未能删除')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: '撤销' })).toHaveLength(1)
    expect(await toolbar().findByText('已选 1 封')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '撤销' }))
    await waitFor(() => expect(client.undoDelete).toHaveBeenCalledWith('batch-9'))
  })

  it('asks once, with a count, when some picked messages would be deleted for good', async () => {
    const { client, user } = await setup([email('a'), email('b', { folderId: trash.id }), email('c', { folderId: trash.id })])
    await user.click(screen.getByRole('button', { name: '选择' }))
    for (const row of rows()) await user.click(row)
    await user.click(toolbar().getByRole('button', { name: '删除' }))
    const dialog = within(await screen.findByRole('alertdialog'))
    expect(dialog.getByText('删除 3 封邮件？')).toBeInTheDocument()
    expect(dialog.getByText(/其中 2 封将被永久删除/)).toBeInTheDocument()
    expect(client.deleteEmails).not.toHaveBeenCalled()
    await user.click(dialog.getByRole('button', { name: '删除' }))
    await waitFor(() => expect(client.deleteEmails).toHaveBeenCalledWith(['a', 'b', 'c'], true, true))
  })

  it('never sends local copies to delete', async () => {
    const { client, user } = await setup([email('a'), email('local', { localOnly: true })])
    await user.click(screen.getByRole('button', { name: '选择' }))
    for (const row of rows()) await user.click(row)
    await user.click(toolbar().getByRole('button', { name: '删除' }))
    await waitFor(() => expect(client.deleteEmails).toHaveBeenCalledWith(['a'], false, true))
  })

  it('cannot archive outside the inbox and says why', async () => {
    const sent = folder('sent', 'sent')
    const { client, user } = await setup([email('a', { folderId: sent.id })], [inbox, sent, trash])
    client.listEmails.mockResolvedValue(page([email('a', { folderId: sent.id })]))
    await user.click(screen.getByRole('button', { name: '选择' }))
    await user.click(rows()[0])
    expect(toolbar().getByRole('button', { name: '归档' })).toBeDisabled()
    expect(toolbar().getByText('已选 1 封')).toBeInTheDocument()
  })

  it('archives the picked inbox messages in one partial call with undo', async () => {
    const { client, user } = await setup([email('a'), email('b')])
    client.archiveEmails.mockResolvedValue({ processedCount: 2, changedCount: 2, undoToken: 'archive-9', skippedIds: [] })
    await user.click(screen.getByRole('button', { name: '选择' }))
    for (const row of rows()) await user.click(row)
    await user.click(toolbar().getByRole('button', { name: '归档' }))
    await waitFor(() => expect(client.archiveEmails).toHaveBeenCalledWith(['a', 'b'], true))
    expect(await screen.findByText('已归档 2 封')).toBeInTheDocument()
    await act(async () => { await user.click(screen.getByRole('button', { name: '撤销' })) })
    await waitFor(() => expect(client.undoArchive).toHaveBeenCalledWith('archive-9'))
  })
})
