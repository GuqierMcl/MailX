import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { ConversationPage, EmailDetail, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { MailxClientError } from '@/lib/mailx-client'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

const account = exampleAccount({ emailAddress: 'Work@Example.com' })
const inbox: Folder = {
  id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox',
  selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0, archivedCount: 0, archivedUnreadCount: 0,
}

function email(id: string, hour: number, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    senderGroup: null,
    id, accountId: account.id, folderId: inbox.id, subject: id === 'm1' ? '团建地点' : `回复：团建地点`,
    from: { name: '王芳', address: 'wang@example.com' }, preview: `预览 ${id}`,
    receivedAt: `2026-10-18T${String(hour).padStart(2, '0')}:00:00.000Z`, isRead: true, isStarred: false,
    hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null,
    to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

const mine = { name: '我自己', address: 'work@example.com' }

const detail = (summary: EmailSummary): EmailDetail => ({
  summary, to: [], cc: [], replyTo: [], attachments: [], bodyFailure: null,
  body: { text: `正文 ${summary.id}`, hasHtml: false, truncated: false, remoteImagesBlocked: 0 },
})

const page = (items: EmailSummary[]): EmailPage => ({
  items, nextCursor: null,
  coverage: { headersComplete: true, bodiesComplete: false, indexedBodies: 0, syncedAfter: '2026-09-18', syncedBefore: null, windowCapped: false },
})

function setup(thread: EmailSummary[], listed: EmailSummary, options: { possiblyIncomplete?: boolean } = {}) {
  const client = testClient([account])
  client.listFolders.mockResolvedValue([inbox])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState('inbox')] })])
  client.listEmails.mockResolvedValue(page([listed]))
  client.getEmail.mockImplementation(async (id) => {
    const found = thread.find((item) => item.id === id) ?? listed
    return detail(found)
  })
  client.getConversation.mockImplementation(async (): Promise<ConversationPage> => ({
    messages: thread, possiblyIncomplete: options.possiblyIncomplete ?? false,
  }))
  return client
}

async function open(client: ReturnType<typeof setup>) {
  render(withTitleBar(<Workbench client={client} />))
  const list = await screen.findByRole('list', { name: '邮件列表' })
  await userEvent.click(within(list).getAllByRole('listitem')[0])
}

/** Collapsed rows in the reader; the list may show the same message as a row of its own. */
const collapsedRows = (id: string) => screen.queryAllByRole('button', { name: new RegExp(`预览 ${id}$`) })
  .filter((button) => !button.closest('[aria-label="邮件列表"]'))

const threeMessages = () => [
  email('m1', 8),
  email('m2', 9, { from: mine, subject: 'Re: 团建地点' }),
  email('m3', 10, { isRead: false }),
]

describe('conversation reader', () => {
  it('shows the thread header, count, collapsed rows and the opened message', async () => {
    const thread = threeMessages()
    await open(setup(thread, thread[2]))
    expect(await screen.findByRole('tablist', { name: '阅读方式' })).toBeInTheDocument()
    expect(screen.getByText('3 封')).toBeInTheDocument()
    expect(screen.getByText('团建地点', { selector: 'p' })).toBeInTheDocument()
    await waitFor(() => expect(collapsedRows('m1')).toHaveLength(1))
    expect(collapsedRows('m1')[0]).toHaveAccessibleName(/^王芳，.*：预览 m1$/)
    expect(collapsedRows('m2')[0]).toHaveAccessibleName(/^我，.*：预览 m2$/)
    // The opened one is expanded: its body shows, and it has no collapsed row.
    expect(await screen.findByText('正文 m3')).toBeInTheDocument()
    expect(collapsedRows('m3')).toHaveLength(0)
  })

  it('expands a row through the normal open path and keeps the same thread', async () => {
    const thread = threeMessages()
    thread[0] = { ...thread[0], isRead: false }
    const client = setup(thread, thread[2])
    await open(client)
    await screen.findByText('正文 m3')
    const asked = client.getConversation.mock.calls.length
    await userEvent.click(await screen.findByRole('button', { name: /^王芳，.*，未读：预览 m1$/ }))
    expect(await screen.findByText('正文 m1')).toBeInTheDocument()
    await waitFor(() => expect(client.setFlag).toHaveBeenCalledWith('m1', 'read', true))
    expect(screen.getByText('3 封')).toBeInTheDocument()
    expect(client.getConversation.mock.calls.slice(0, asked).every(([id]) => id === 'm3')).toBe(true)
    expect(client.getConversation.mock.calls.every(([id]) => id === 'm3')).toBe(true)
    // The previously expanded message is now a collapsed row.
    await waitFor(() => expect(collapsedRows('m3')).toHaveLength(1))
  })

  it('collapses rows beyond three on a side into one row that reveals them', async () => {
    const thread = [email('m1', 1), email('m2', 2), email('m3', 3), email('m4', 4), email('m5', 5), email('m6', 6)]
    await open(setup(thread, thread[5]))
    await screen.findByText('正文 m6')
    expect(collapsedRows('m1')).toHaveLength(0)
    expect(collapsedRows('m2')).toHaveLength(0)
    expect(collapsedRows('m3')).toHaveLength(1)
    await userEvent.click(screen.getByRole('button', { name: '更早的 2 封' }))
    expect(collapsedRows('m1')).toHaveLength(1)
    expect(screen.queryByRole('button', { name: '更早的 2 封' })).not.toBeInTheDocument()
  })

  it('switches to the single reader and back', async () => {
    const thread = threeMessages()
    await open(setup(thread, thread[2]))
    await screen.findByText('正文 m3')
    await userEvent.click(screen.getByRole('tab', { name: '单封' }))
    expect(collapsedRows('m1')).toHaveLength(0)
    expect(screen.getByText('正文 m3')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('tab', { name: '对话' }))
    await waitFor(() => expect(collapsedRows('m1')).toHaveLength(1))
  })

  it('keeps the normal reader for a one-message thread', async () => {
    const only = email('m1', 8)
    await open(setup([only], only))
    expect(await screen.findByText('正文 m1')).toBeInTheDocument()
    expect(screen.queryByRole('tablist', { name: '阅读方式' })).not.toBeInTheDocument()
    expect(screen.queryByText('1 封')).not.toBeInTheDocument()
  })

  it('keeps the normal reader when the thread cannot be read', async () => {
    const thread = threeMessages()
    const client = setup(thread, thread[2])
    client.getConversation.mockRejectedValue(new MailxClientError({ code: 'notFound', message: '内容不存在。', field: null }))
    await open(client)
    expect(await screen.findByText('正文 m3')).toBeInTheDocument()
    expect(screen.queryByRole('tablist', { name: '阅读方式' })).not.toBeInTheDocument()
  })

  it('keeps the normal reader for a local-only copy', async () => {
    const thread = threeMessages()
    thread[2] = { ...thread[2], localOnly: true }
    await open(setup(thread, thread[2]))
    expect(await screen.findByText('正文 m3')).toBeInTheDocument()
    expect(screen.queryByRole('tablist', { name: '阅读方式' })).not.toBeInTheDocument()
  })

  it('says quietly when earlier messages may be missing', async () => {
    const thread = threeMessages()
    await open(setup(thread, thread[2], { possiblyIncomplete: true }))
    expect(await screen.findByText('本机可能没有这段对话的全部邮件')).toBeInTheDocument()
  })
})
