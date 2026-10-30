import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { EmailDetail, EmailPage, EmailSummary, Folder, OutboxChanged, SentCopiesChanged } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { MailxClientError } from '@/lib/mailx-client'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

const account = exampleAccount()
const folders: Folder[] = [
  { id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox', selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0 },
  { id: 'sent', accountId: account.id, parentId: null, name: 'Sent', role: 'sent', selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0 },
]

const localCopy: EmailSummary = {
  id: 'local-sent:1', accountId: account.id, folderId: 'sent', subject: '周五评审安排', preview: '',
  from: { name: null, address: 'me@example.com' }, receivedAt: '2026-10-28T08:00:00.000Z',
  isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false,
  to: { addresses: [{ name: null, address: 'chen@example.com' }], total: 3 }, localOnly: true,
}

const page = (items: EmailSummary[]): EmailPage => ({
  items, nextCursor: null,
  coverage: { headersComplete: true, bodiesComplete: false, syncedAfter: '2026-09-18', syncedBefore: null, windowCapped: false },
})

const detail = (summary: EmailSummary): EmailDetail => ({
  summary, to: summary.to.addresses, cc: [], replyTo: [], attachments: [], bodyFailure: null,
  body: { text: '会议室已订好', hasHtml: false, truncated: false, remoteImagesBlocked: 0 },
})

function setup() {
  let stored: EmailSummary[] = [localCopy]
  const client = testClient([account])
  client.listFolders.mockResolvedValue(folders)
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState('inbox'), exampleFolderSyncState('sent')] })])
  client.listEmails.mockImplementation(async (query) => page(query.folderRole === 'inbox' ? [] : stored))
  client.getEmail.mockImplementation(async (id) => {
    const found = stored.find((item) => item.id === id)
    if (!found) throw new MailxClientError({ code: 'notFound', message: '内容不存在。', field: null })
    return detail(found)
  })
  let outbox: (change: OutboxChanged) => void = () => undefined
  client.onOutbox.mockImplementation((listener) => {
    outbox = listener
    return () => undefined
  })
  let sentCopies: (change: SentCopiesChanged) => void = () => undefined
  client.onSentCopiesChanged.mockImplementation((listener) => {
    sentCopies = listener
    return () => undefined
  })
  const change = (): OutboxChanged => ({ accountId: account.id, outboxId: 'o1', draftId: null, state: 'accepted', attempts: 0, nextAttemptAt: null, errorReason: null })
  return { client, drop: () => { stored = [] }, emitOutbox: () => act(() => outbox(change())),
    emitSentCopies: () => act(() => sentCopies({ accountId: account.id })) }
}

async function openSentCopy(client: ReturnType<typeof setup>['client']) {
  render(withTitleBar(<Workbench client={client} />))
  await userEvent.click((await screen.findAllByRole('button', { name: '已发送' }))[0])
  const list = await screen.findByRole('list', { name: '邮件列表' })
  const row = within(list).getAllByRole('listitem')[0]
  expect(row).toHaveTextContent('致：chen@example.com 等 3 人')
  await userEvent.click(row)
  await screen.findByRole('heading', { level: 2, name: localCopy.subject })
}

describe('local sent copies in the Sent folder', () => {
  it('removes a copy after confirming, refreshes the lists and closes the reader', async () => {
    const { client, drop } = setup()
    client.removeLocalCopy.mockImplementation(async () => { drop() })
    await openSentCopy(client)
    expect(screen.queryByRole('button', { name: /标为(已读|未读)/ })).not.toBeInTheDocument()
    const before = client.listEmails.mock.calls.length
    await userEvent.click(screen.getByRole('button', { name: '从本机移除' }))
    await userEvent.click(await screen.findByRole('button', { name: '移除' }))
    expect(client.removeLocalCopy).toHaveBeenCalledExactlyOnceWith(localCopy.id)
    await waitFor(() => expect(client.listEmails.mock.calls.length).toBeGreaterThan(before))
    await waitFor(() => expect(screen.queryByRole('heading', { level: 2, name: localCopy.subject })).not.toBeInTheDocument())
    expect(screen.getAllByText('未选择邮件').length).toBeGreaterThan(0)
  })

  it('keeps the reader and shows the error when the removal fails', async () => {
    const { client } = setup()
    client.removeLocalCopy.mockRejectedValue(new MailxClientError({ code: 'notFound', message: '内容不存在。', field: null }))
    await openSentCopy(client)
    await userEvent.click(screen.getByRole('button', { name: '从本机移除' }))
    await userEvent.click(await screen.findByRole('button', { name: '移除' }))
    expect(await screen.findByText(/内容不存在/)).toBeInTheDocument()
  })

  it('refetches on an outbox event and closes the reader once the copy is gone', async () => {
    const { client, drop, emitOutbox } = setup()
    await openSentCopy(client)
    const before = client.listEmails.mock.calls.length
    // The copy was matched to its server message and replaced.
    drop()
    await emitOutbox()
    await waitFor(() => expect(client.listEmails.mock.calls.length).toBeGreaterThan(before))
    await waitFor(() => expect(screen.getAllByText('未选择邮件').length).toBeGreaterThan(0))
    expect(screen.queryByRole('button', { name: '关闭邮件' })).not.toBeInTheDocument()
  })

  it('refetches on a sent-copies event and closes a vanished reader', async () => {
    const { client, drop, emitSentCopies } = setup()
    await openSentCopy(client)
    const before = client.listEmails.mock.calls.length
    drop()
    await emitSentCopies()
    await waitFor(() => expect(client.listEmails.mock.calls.length).toBeGreaterThan(before))
    await waitFor(() => expect(screen.getAllByText('未选择邮件').length).toBeGreaterThan(0))
  })

  it('shows no sent toast when a copy is removed', async () => {
    const { client, drop, emitSentCopies } = setup()
    client.removeLocalCopy.mockImplementation(async () => { drop(); await emitSentCopies() })
    await openSentCopy(client)
    await userEvent.click(screen.getByRole('button', { name: '从本机移除' }))
    await userEvent.click(await screen.findByRole('button', { name: '移除' }))
    await waitFor(() => expect(screen.queryByRole('heading', { level: 2, name: localCopy.subject })).not.toBeInTheDocument())
    // "已发送" remains as the sidebar button and the list title; a toast would be other plain text.
    expect(screen.queryAllByText('已发送').filter((node) => !node.closest('button, h1'))).toEqual([])
  })
})
