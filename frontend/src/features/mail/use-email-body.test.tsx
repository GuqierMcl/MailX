import { StrictMode, type ReactNode } from 'react'
import { QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { EmailDetail } from '@/generated/accounts'
import { exampleAccount, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { createQueryClient, mailKeys } from '@/lib/query-client'
import { bodyIdentity, useEmailBody } from './use-email-body'

const account = exampleAccount()
const state = exampleSyncState()
function mail(id = 'one', ready = false): EmailDetail {
  return {
    summary: { id, accountId: account.id, folderId: 'inbox', subject: id, from: { name: null, address: 'test@example.com' },
      receivedAt: '2026-10-20T00:00:00Z', isRead: false, isStarred: false, pendingChanges: false, hasAttachment: false,
      bodyState: ready ? 'ready' : 'missing', preview: ready ? 'cached preview' : '' },
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
  it('downloads once in StrictMode, keeps ready metadata over old headers, and never retries failed DTOs automatically', async () => {
    const { client, wrapper } = setup()
    client.fetchBody.mockResolvedValueOnce({ ...mail(), summary: { ...mail().summary, bodyState: 'failed' }, bodyFailure: 'network' })
    const { result, rerender } = renderHook(({ id }) => useEmailBody(client, mail(id).summary, [account], { [account.id]: state }), { wrapper, initialProps: { id: 'one' } })
    await waitFor(() => expect(result.current.email?.bodyFailure).toBe('network'))
    rerender({ id: 'two' })
    await waitFor(() => expect(result.current.email?.summary.bodyState).toBe('failed'))
    rerender({ id: 'one' })
    await waitFor(() => expect(result.current.email?.bodyFailure).toBe('network'))
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
    const { result } = renderHook(() => useEmailBody(client, mail().summary, [account], { [account.id]: state }), { wrapper })
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
    const { result, rerender } = renderHook(({ id }) => useEmailBody(client, mail(id).summary, [account], { [account.id]: state }), { wrapper, initialProps: { id: 'one' } })
    await waitFor(() => expect(client.fetchBody).toHaveBeenCalledTimes(1))
    rerender({ id: 'two' })
    await waitFor(() => expect(result.current.email?.summary.bodyState).toBe('ready'))
    await act(async () => first.resolve(mail('one', true)))
    expect(result.current.email?.summary.id).toBe('two')
    rerender({ id: 'one' })
    await waitFor(() => expect(result.current.email?.summary.bodyState).toBe('ready'))
    expect(client.fetchBody).toHaveBeenCalledTimes(2)
  })
})
