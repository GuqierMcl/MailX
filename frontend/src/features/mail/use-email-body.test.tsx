import { StrictMode, type ReactNode } from 'react'
import { QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { EmailDetail } from '@/generated/accounts'
import { exampleAccount, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { createQueryClient, mailKeys } from '@/lib/query-client'
import type { MailxClient } from '@/lib/mailx-client'
import { clearRemovedAccount } from './account-removal'
import { bodyIdentity, useEmailBody } from './use-email-body'

const account = exampleAccount()
const state = exampleSyncState()
function mail(id = 'one', ready = false): EmailDetail {
  return {
    summary: { id, accountId: account.id, folderId: 'inbox', subject: id, from: { name: null, address: 'test@example.com' },
      receivedAt: '2026-10-20T00:00:00Z', isRead: false, isStarred: false, pendingChanges: false, hasAttachment: false,
      bodyState: ready ? 'ready' : 'missing', previewState: ready ? 'ready' : 'missing', preview: ready ? 'cached preview' : '' },
    to: [], cc: [], replyTo: [], attachments: [], bodyFailure: null,
    body: ready ? { text: 'cached text', hasHtml: false, truncated: false, remoteImagesBlocked: 0 } : null,
  }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}
function setup() {
  const client = testClient([account])
  client.getEmail.mockImplementation(async (id) => mail(id))
  const cache = createQueryClient()
  const wrapper = ({ children }: { children: ReactNode }) => <StrictMode><QueryClientProvider client={cache}>{children}</QueryClientProvider></StrictMode>
  return { client, cache, wrapper }
}

describe('on-demand body queries', () => {
  it('withdraws a download immediately when its account is removed before the next tree render', async () => {
    const { client, cache, wrapper } = setup()
    const listeners = new Set<Parameters<MailxClient['onAccountRemoved']>[0]>()
    client.onAccountRemoved.mockImplementation((listener) => { listeners.add(listener); return () => { listeners.delete(listener) } })
    const download = deferred<EmailDetail>()
    client.fetchBody.mockReturnValue(download.promise)
    const { result } = renderHook(() => useEmailBody(client, mail().summary, [account], { [account.id]: state }), { wrapper })
    await waitFor(() => expect(client.fetchBody).toHaveBeenCalledTimes(1))
    await act(async () => {
      const report = { accountId: account.id, draftIds: [], credentialsPending: false }
      listeners.forEach((listener) => listener(report))
      clearRemovedAccount(cache, report)
      download.resolve(mail('one', true))
    })
    expect(result.current.identity).toBe('')
    expect(cache.getQueriesData({ queryKey: mailKeys.email('one') }).some(([, data]) => (data as EmailDetail | undefined)?.body?.text === 'cached text')).toBe(false)
  })

  it.each(['network', 'providerPolicy'] as const)('downloads once in StrictMode, preserves %s without automatic retries, and keeps a manual retry over old headers', async (bodyFailure) => {
    const { client, wrapper } = setup()
    client.fetchBody.mockResolvedValueOnce({ ...mail(), summary: { ...mail().summary, bodyState: 'failed' }, bodyFailure })
    const { result, rerender } = renderHook(({ id }) => useEmailBody(client, { id, accountId: account.id }, [account], { [account.id]: state }), { wrapper, initialProps: { id: 'one' } })
    await waitFor(() => expect(result.current.email?.bodyFailure).toBe(bodyFailure))
    rerender({ id: 'two' })
    await waitFor(() => expect(result.current.email?.summary.bodyState).toBe('failed'))
    rerender({ id: 'one' })
    await waitFor(() => expect(result.current.email?.bodyFailure).toBe(bodyFailure))
    expect(client.fetchBody.mock.calls.filter(([id]) => id === 'one')).toHaveLength(1)
    client.fetchBody.mockResolvedValue(mail('one', true))
    act(() => { result.current.download(); result.current.download() })
    await waitFor(() => expect(result.current.email?.body?.text).toBe('cached text'))
    expect(result.current.email?.summary.bodyState).toBe('ready')
    expect(result.current.email?.summary.preview).toBe('cached preview')
    expect(client.fetchBody.mock.calls.filter(([id]) => id === 'one')).toHaveLength(2)
  })

  it('cancels an older detail refetch before writing the downloaded result', async () => {
    const { client, cache, wrapper } = setup()
    const download = deferred<EmailDetail>()
    client.fetchBody.mockReturnValue(download.promise)
    const { result } = renderHook(() => useEmailBody(client, { id: 'one', accountId: account.id }, [account], { [account.id]: state }), { wrapper })
    await waitFor(() => expect(client.fetchBody).toHaveBeenCalledTimes(1))
    const stale = deferred<EmailDetail>()
    client.getEmail.mockReturnValue(stale.promise)
    void cache.invalidateQueries({ queryKey: mailKeys.email('one') })
    await waitFor(() => expect(client.getEmail).toHaveBeenCalledTimes(2))
    await act(async () => download.resolve(mail('one', true)))
    await waitFor(() => expect(result.current.email?.body).not.toBeNull())
    await act(async () => stale.resolve(mail()))
    expect(result.current.email?.summary.bodyState).toBe('ready')
  })

  it.each(['revision', 'source', 'generation'] as const)('discards a late download after %s changes', async (change) => {
    const { client, cache, wrapper } = setup()
    const old = deferred<EmailDetail>()
    client.fetchBody.mockReturnValueOnce(old.promise).mockResolvedValue({ ...mail('one', true), body: { text: 'current', hasHtml: false, truncated: false, remoteImagesBlocked: 0 } })
    const { result, rerender } = renderHook(({ account: current, sync }) => useEmailBody(client, mail().summary, [current], { [current.id]: sync }), { wrapper, initialProps: { account, sync: state } })
    await waitFor(() => expect(client.fetchBody).toHaveBeenCalledTimes(1))
    const next = { ...account, revision: change === 'revision' ? account.revision + 1 : account.revision, mailboxSourceId: change === 'source' ? 'replacement' : account.mailboxSourceId }
    rerender({ account: next, sync: { ...state, mailboxSourceId: next.mailboxSourceId, generation: change === 'generation' ? 2 : state.generation } })
    await waitFor(() => expect(result.current.email?.body?.text).toBe('current'))
    await act(async () => old.resolve(mail('one', true)))
    expect(result.current.email?.body?.text).toBe('current')
    expect(cache.getQueryData<EmailDetail>([...mailKeys.email('one'), bodyIdentity(account, state)])?.body).toBeNull()
  })

  it('keeps different messages independent while downloads overlap', async () => {
    const { client, wrapper } = setup()
    const first = deferred<EmailDetail>()
    client.fetchBody.mockImplementation((id) => id === 'one' ? first.promise : Promise.resolve(mail(id, true)))
    const { result, rerender } = renderHook(({ id }) => useEmailBody(client, { id, accountId: account.id }, [account], { [account.id]: state }), { wrapper, initialProps: { id: 'one' } })
    await waitFor(() => expect(client.fetchBody).toHaveBeenCalledTimes(1))
    rerender({ id: 'two' })
    await waitFor(() => expect(result.current.email?.summary.bodyState).toBe('ready'))
    await act(async () => first.resolve(mail('one', true)))
    expect(result.current.email?.summary.id).toBe('two')
    rerender({ id: 'one' })
    await waitFor(() => expect(result.current.email?.summary.bodyState).toBe('ready'))
    expect(client.fetchBody).toHaveBeenCalledTimes(2)
  })

  it('shows the message\'s own flags, so a message missing from every list still reads correctly', async () => {
    const { client, wrapper } = setup()
    client.getEmail.mockImplementation(async (id) => ({
      ...mail(id, true), summary: { ...mail(id, true).summary, isRead: true, isStarred: true, pendingChanges: true },
    }))
    const { result } = renderHook(() => useEmailBody(client, { id: 'only-here', accountId: account.id }, [account], { [account.id]: state }), { wrapper })
    await waitFor(() => expect(result.current.email).not.toBeNull())
    expect(result.current.email?.summary).toMatchObject({ isRead: true, isStarred: true, pendingChanges: true })
  })

  it('opens nothing without a message', () => {
    const { client, wrapper } = setup()
    const { result } = renderHook(() => useEmailBody(client, null, [account], { [account.id]: state }), { wrapper })
    expect(result.current.email).toBeNull()
    expect(client.getEmail).not.toHaveBeenCalled()
  })
})
