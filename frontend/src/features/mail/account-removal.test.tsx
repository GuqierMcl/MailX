import { QueryClientProvider } from '@tanstack/react-query'
import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { AccountSyncState, EmailDetail, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import type { MailxClient } from '@/lib/mailx-client'
import { composeKeys, createQueryClient, mailKeys } from '@/lib/query-client'
import { withTitleBar } from '@/test/title-bar'
import { clearRemovedAccount } from './account-removal'
import { bodyIdentity } from './use-email-body'
import { Workbench } from './workbench'
import { emailQuery } from './format'

const account = exampleAccount()
const other = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人邮箱', mailboxSourceId: 'source-b' })
const rows = [account, other]
const folder = (accountId: string): Folder => ({
  id: `${accountId}-inbox`, accountId, parentId: null, name: 'INBOX', role: 'inbox', selectable: true,
  countsKnown: true, unreadCount: 0, totalCount: 1,
})
function email(accountId: string): EmailSummary {
  return {
    id: `${accountId}-mail`, accountId, folderId: folder(accountId).id, subject: accountId === account.id ? 'Removed message' : 'Surviving message',
    from: { name: 'Sender', address: 'sender@example.com' }, preview: 'Preview', receivedAt: '2026-10-18T08:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false,
  }
}
function page(items: EmailSummary[]): EmailPage {
  return { items, nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }
}
function detail(accountId: string): EmailDetail {
  return { summary: email(accountId), to: [], cc: [], replyTo: [], attachments: [], bodyFailure: null,
    body: { text: `${accountId} body`, hasHtml: false, truncated: false, remoteImagesBlocked: 0 } }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

function setup(rows = [account, other]) {
  const client = testClient(rows)
  const cache = createQueryClient()
  let current = rows
  client.listAccounts.mockImplementation(async () => current)
  client.listFolders.mockImplementation(async (id) => [folder(id)])
  client.listEmails.mockImplementation(async (query) => page(current.map((row) => email(row.id))
    .filter((item) => !query.folderId || item.folderId === query.folderId)))
  client.getEmail.mockImplementation(async (id) => detail(id === email(account.id).id ? account.id : other.id))
  const removals = new Set<Parameters<MailxClient['onAccountRemoved']>[0]>()
  client.onAccountRemoved.mockImplementation((listener) => { removals.add(listener); return () => { removals.delete(listener) } })
  let sync: (state: AccountSyncState) => void = () => undefined
  client.onSyncState.mockImplementation((listener) => { sync = listener; return () => undefined })
  const remove = () => act(() => {
    current = current.filter((row) => row.id !== account.id)
    removals.forEach((listener) => listener({ accountId: account.id, draftIds: ['removed-draft'], credentialsPending: false }))
  })
  const emitStaleSync = () => act(() => sync(exampleSyncState({ sequence: 900, folderRevision: 900 })))
  const view = render(withTitleBar(<QueryClientProvider client={cache}><Workbench client={client} /></QueryClientProvider>))
  return { client, cache, remove, emitStaleSync, view }
}

describe('workbench account removal', () => {
  it('falls back from the removed folder, closes its reader, and keeps the other account', async () => {
    const { client, cache, remove, emitStaleSync } = setup()
    await screen.findByRole('list', { name: '邮件列表' })
    await userEvent.click(screen.getAllByRole('button', { name: '收件箱' })[0])
    const list = await screen.findByRole('list', { name: '邮件列表' })
    await waitFor(() => expect(within(list).getAllByRole('listitem')).toHaveLength(1))
    await userEvent.click(within(list).getByRole('listitem'))
    await screen.findByRole('heading', { level: 2, name: 'Removed message' })
    remove()
    expect(await screen.findByRole('heading', { name: '全部收件箱' })).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('list', { name: '邮件列表' })).toHaveTextContent('Surviving message'))
    expect(screen.queryByText('Removed message')).not.toBeInTheDocument()
    expect(screen.queryByRole('heading', { level: 2, name: 'Removed message' })).not.toBeInTheDocument()
    expect(cache.getQueryData<{ accounts: typeof rows }>(mailKeys.tree())?.accounts.map((row) => row.id)).toEqual([other.id])
    expect(cache.getQueriesData({ queryKey: mailKeys.email(email(account.id).id) })).toEqual([])
    const refreshed = client.listAccounts.mock.calls.length
    emitStaleSync()
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 300)) })
    expect(client.listAccounts).toHaveBeenCalledTimes(refreshed)
  })

  it('shows the add-account empty state after removing the final account', async () => {
    const { client, remove } = setup([account])
    await screen.findByRole('list', { name: '邮件列表' })
    remove()
    expect(await screen.findByRole('button', { name: '添加邮箱账号' })).toBeInTheDocument()
    expect(screen.queryByRole('list', { name: '邮件列表' })).not.toBeInTheDocument()
    expect(client.listDrafts.mock.calls.length).toBeGreaterThan(1)
    expect(client.listOutbox.mock.calls.length).toBeGreaterThan(1)
  })

  it('ignores a stale tree response that was already loading when removal completed', async () => {
    const { client, cache, remove } = setup()
    await screen.findByRole('list', { name: '邮件列表' })
    const stale = deferred<typeof rows>()
    client.listAccounts.mockReturnValueOnce(stale.promise)
    void cache.invalidateQueries({ queryKey: mailKeys.tree() })
    await waitFor(() => expect(client.listAccounts).toHaveBeenCalledTimes(2))
    remove()
    await act(async () => stale.resolve([account, other]))
    await waitFor(() => expect(cache.getQueryData<{ accounts: typeof rows }>(mailKeys.tree())?.accounts.map((row) => row.id)).toEqual([other.id]))
    expect(screen.queryByText('Removed message')).not.toBeInTheDocument()
  })
})

describe('removed account query cleanup', () => {
  it('purges account data from list pages, palette, body and compose caches while preserving other accounts', () => {
    const cache = createQueryClient()
    const listKey = mailKeys.emails('sources', emailQuery({ folderRole: 'inbox', limit: 100 }))
    cache.setQueryData(mailKeys.tree(), { accounts: rows, folders: rows.map((row) => folder(row.id)) })
    cache.setQueryData(listKey, { pages: [page(rows.map((row) => email(row.id)))], pageParams: [null] })
    const paletteKey = [...mailKeys.all, 'palette', 'sources', 'search']
    cache.setQueryData(paletteKey, page(rows.map((row) => email(row.id))))
    cache.setQueryData([...mailKeys.email(email(account.id).id), bodyIdentity(account)], detail(account.id))
    const otherBody = [...mailKeys.email(email(other.id).id), bodyIdentity(other)]
    cache.setQueryData(otherBody, detail(other.id))
    cache.setQueryData(composeKeys.drafts(), [{ id: 'removed-draft', accountId: account.id }, { id: 'kept-draft', accountId: other.id }])
    cache.setQueryData([...composeKeys.drafts(), 'removed-draft'], { id: 'removed-draft', accountId: account.id })
    cache.setQueryData(composeKeys.outbox(), [{ id: 'removed-send', accountId: account.id }, { id: 'kept-send', accountId: other.id }])
    clearRemovedAccount(cache, { accountId: account.id, draftIds: ['removed-draft'] })
    expect(cache.getQueryData<{ pages: EmailPage[] }>(listKey)?.pages[0].items.map((item) => item.accountId)).toEqual([other.id])
    expect(cache.getQueryData<EmailPage>(paletteKey)?.items.map((item) => item.accountId)).toEqual([other.id])
    expect(cache.getQueriesData({ queryKey: mailKeys.email(email(account.id).id) })).toEqual([])
    expect(cache.getQueryData(otherBody)).toEqual(detail(other.id))
    expect(cache.getQueryData(composeKeys.drafts())).toEqual([{ id: 'kept-draft', accountId: other.id }])
    expect(cache.getQueryData([...composeKeys.drafts(), 'removed-draft'])).toBeUndefined()
    expect(cache.getQueryData(composeKeys.outbox())).toEqual([{ id: 'kept-send', accountId: other.id }])
  })
})
