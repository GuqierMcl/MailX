import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Draft } from '@/generated/accounts'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { AUTOSAVE_DELAY_MS, useDraftEditor } from './use-draft-editor'

const account = exampleAccount()
const draft: Draft = {
  id: 'draft-one', accountId: account.id, kind: 'new', sourceEmailId: null,
  to: [], cc: [], bcc: [], subject: 'Original', bodyHtml: '<p>Original</p>', bodyText: 'Original',
  inReplyTo: null, references: null, createdAt: '2026-10-23T00:00:00Z', updatedAt: '2026-10-23T00:00:00Z', revision: 1,
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (cause: unknown) => void
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}

function setup() {
  const client = testClient([account])
  client.getDraft.mockResolvedValue(draft)
  client.updateDraft.mockImplementation(async (request) => ({ ...draft, ...request, accountId: request.accountId ?? draft.accountId, revision: request.expectedRevision + 1 }))
  const listeners = new Set<Parameters<MailxClient['onAccountRemoved']>[0]>()
  client.onAccountRemoved.mockImplementation((listener) => { listeners.add(listener); return () => { listeners.delete(listener) } })
  const remove = (draftIds = [draft.id]) => act(() => {
    listeners.forEach((listener) => listener({ accountId: account.id, draftIds, credentialsPending: false }))
  })
  return { client, remove }
}

afterEach(() => vi.useRealTimers())

describe('draft editor account removal', () => {
  it('rejects a late initial load after the draft was removed', async () => {
    const { client, remove } = setup()
    const load = deferred<Draft>()
    client.getDraft.mockReturnValue(load.promise)
    const { result } = renderHook(() => useDraftEditor(client, draft.id))
    remove()
    await act(async () => load.resolve(draft))
    expect(result.current.gone).toBe(true)
    expect(result.current.fields).toBeNull()
    expect(result.current.isAvailable()).toBe(false)
  })

  it('cancels pending autosave and makes flush and baseline restore inert', async () => {
    const { client, remove } = setup()
    const { result } = renderHook(() => useDraftEditor(client, draft.id))
    await waitFor(() => expect(result.current.fields).not.toBeNull())
    vi.useFakeTimers()
    act(() => result.current.update({ subject: 'Unsaved' }))
    remove()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS + 1)
      expect(await result.current.flush()).toEqual({ ok: false })
      await result.current.restoreBaseline()
      await result.current.remove()
      result.current.retry()
    })
    expect(client.updateDraft).not.toHaveBeenCalled()
    expect(client.deleteDraft).not.toHaveBeenCalled()
  })

  it('does not reload or restore the draft when an in-flight save rejects after removal', async () => {
    const { client, remove } = setup()
    const save = deferred<Draft>()
    client.updateDraft.mockReturnValue(save.promise)
    const notice = vi.fn()
    const { result } = renderHook(() => useDraftEditor(client, draft.id, notice))
    await waitFor(() => expect(result.current.fields).not.toBeNull())
    act(() => result.current.update({ subject: 'Unsaved' }))
    let flushed!: ReturnType<typeof result.current.flush>
    let restored!: ReturnType<typeof result.current.restoreBaseline>
    act(() => { flushed = result.current.flush(); restored = result.current.restoreBaseline() })
    remove()
    await act(async () => {
      save.reject(new MailxClientError({ code: 'conflict', message: 'x', field: null }))
      expect(await flushed).toEqual({ ok: false })
      await restored
    })
    expect(client.updateDraft).toHaveBeenCalledTimes(1)
    expect(client.getDraft).toHaveBeenCalledTimes(1)
    expect(notice).not.toHaveBeenCalled()
    expect(result.current.gone).toBe(true)
  })

  it('does not accept canonical content from a save that completes after removal', async () => {
    const { client, remove } = setup()
    const save = deferred<Draft>()
    client.updateDraft.mockReturnValue(save.promise)
    const { result } = renderHook(() => useDraftEditor(client, draft.id))
    await waitFor(() => expect(result.current.fields).not.toBeNull())
    act(() => result.current.update({ bodyHtml: '<p>Unsaved</p>' }))
    let flushed!: ReturnType<typeof result.current.flush>
    act(() => { flushed = result.current.flush() })
    remove()
    await act(async () => {
      save.resolve({ ...draft, bodyHtml: '<p>Late canonical body</p>', revision: 2 })
      await flushed
    })
    expect(result.current.fields?.bodyHtml).toBe('<p>Unsaved</p>')
    expect(result.current.revision()).toBe(1)
    expect(result.current.gone).toBe(true)
  })

  it('uses actual deleted draft IDs even when the editor still names the removed account', async () => {
    const { client, remove } = setup()
    const { result } = renderHook(() => useDraftEditor(client, draft.id))
    await waitFor(() => expect(result.current.fields).not.toBeNull())
    remove(['another-draft'])
    act(() => result.current.update({ subject: 'Still editable' }))
    await act(async () => { expect(await result.current.flush()).toEqual({ ok: true }) })
    expect(client.updateDraft).toHaveBeenCalledTimes(1)
    expect(result.current.isAvailable()).toBe(true)
  })
})
