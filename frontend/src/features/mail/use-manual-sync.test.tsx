import { act, renderHook } from '@/test/render'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AccountSyncState } from '@/generated/accounts'
import { exampleSyncState } from '@/features/accounts/test-fixtures'
import { toast } from '@/components/ui/toast'
import { useManualSync, useManualSyncAll } from './use-manual-sync'

vi.mock('@/components/ui/toast', () => ({ toast: { error: vi.fn() } }))

function deferred() {
  let resolve!: () => void
  let reject!: (error: unknown) => void
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

const state = (overrides: Partial<AccountSyncState> = {}) => exampleSyncState(overrides)

function setup(sync: AccountSyncState | undefined = state(), onRefresh = vi.fn<() => Promise<void>>().mockResolvedValue(undefined)) {
  return { onRefresh, ...renderHook<ReturnType<typeof useManualSync>, { sync: AccountSyncState | undefined }>(
    ({ sync }) => useManualSync(sync, onRefresh), { initialProps: { sync } },
  ) }
}

afterEach(() => {
  vi.useRealTimers()
  vi.clearAllMocks()
})

describe('manual account sync feedback', () => {
  it('stays quiet for automatic rounds and prevents refreshing an already syncing account', async () => {
    const view = setup(state({ phase: 'syncing' }))
    expect(view.result.current.refreshing).toBe(false)
    await act(async () => view.result.current.refresh())
    expect(view.onRefresh).not.toHaveBeenCalled()
    view.rerender({ sync: state({ phase: 'idle', sequence: 2 }) })
    expect(view.result.current.refreshing).toBe(false)
  })

  it('waits for a newer completed round, prevents duplicate requests, and stays quiet on the next automatic round', async () => {
    const view = setup()
    await act(async () => {
      view.result.current.refresh()
      view.result.current.refresh()
    })
    expect(view.onRefresh).toHaveBeenCalledTimes(1)
    expect(view.result.current.refreshing).toBe(true)
    // A wake-up response or an unchanged snapshot is not a completed round.
    view.rerender({ sync: state() })
    expect(view.result.current.refreshing).toBe(true)
    view.rerender({ sync: state({ phase: 'syncing', sequence: 2 }) })
    expect(view.result.current.refreshing).toBe(true)
    view.rerender({ sync: state({ phase: 'idle', sequence: 3 }) })
    expect(view.result.current.refreshing).toBe(false)
    view.rerender({ sync: state({ phase: 'syncing', sequence: 4 }) })
    expect(view.result.current.refreshing).toBe(false)
  })

  it('finishes when a terminal event arrives before the request response', async () => {
    const response = deferred()
    const view = setup(state(), vi.fn().mockReturnValue(response.promise))
    await act(async () => view.result.current.refresh())
    view.rerender({ sync: state({ sequence: 2 }) })
    expect(view.result.current.refreshing).toBe(false)
    await act(async () => response.resolve())
    expect(view.result.current.refreshing).toBe(false)
  })

  it('ends on a failed round and lets the account be refreshed again', async () => {
    const view = setup()
    await act(async () => view.result.current.refresh())
    view.rerender({ sync: state({ phase: 'failed', sequence: 2 }) })
    expect(view.result.current.refreshing).toBe(false)
    await act(async () => view.result.current.refresh())
    expect(view.onRefresh).toHaveBeenCalledTimes(2)
    expect(view.result.current.refreshing).toBe(true)
  })

  it('restores the button and reports a rejected wake-up request', async () => {
    const view = setup(state(), vi.fn().mockRejectedValue(new Error('request rejected')))
    await act(async () => view.result.current.refresh())
    expect(view.result.current.refreshing).toBe(false)
    expect(toast.error).toHaveBeenCalledTimes(1)
  })

  it('follows the new worker generation deliberately started by a provider-policy retry', async () => {
    const view = setup(state({
      phase: 'failed',
      failure: { code: 'providerPolicy', message: 'policy', detail: null, willRetry: false },
    }))
    await act(async () => view.result.current.refresh())
    view.rerender({ sync: state({ generation: 2, phase: 'syncing' }) })
    expect(view.result.current.refreshing).toBe(true)
    view.rerender({ sync: state({ generation: 2, sequence: 2 }) })
    expect(view.result.current.refreshing).toBe(false)
  })

  it.each([
    { generation: 2, sequence: 1 },
    { mailboxSourceId: 'replacement-source', sequence: 1 },
  ])('cancels feedback when the worker or server source changes: %j', async (identity) => {
    const view = setup()
    await act(async () => view.result.current.refresh())
    view.rerender({ sync: state({ ...identity, phase: 'syncing' }) })
    expect(view.result.current.refreshing).toBe(false)
  })

  it('stops waiting after 15 seconds if no worker starts, without letting an old rejection end a retry', async () => {
    vi.useFakeTimers()
    const oldResponse = deferred()
    const onRefresh = vi.fn<() => Promise<void>>().mockReturnValueOnce(oldResponse.promise).mockResolvedValue(undefined)
    const view = setup(state(), onRefresh)
    await act(async () => view.result.current.refresh())
    act(() => vi.advanceTimersByTime(15_000))
    expect(view.result.current.refreshing).toBe(false)
    await act(async () => view.result.current.refresh())
    expect(view.result.current.refreshing).toBe(true)
    await act(async () => oldResponse.reject(new Error('obsolete request')))
    expect(view.result.current.refreshing).toBe(true)
    expect(toast.error).not.toHaveBeenCalled()
  })

  it('does not time out a running round, including one started before the first snapshot', async () => {
    vi.useFakeTimers()
    const view = setup(undefined)
    // setup's default makes an idle snapshot; explicitly remove it before clicking.
    view.rerender({ sync: undefined })
    await act(async () => view.result.current.refresh())
    view.rerender({ sync: state({ phase: 'syncing' }) })
    act(() => vi.advanceTimersByTime(60_000))
    expect(view.result.current.refreshing).toBe(true)
    view.rerender({ sync: state({ sequence: 2 }) })
    expect(view.result.current.refreshing).toBe(false)
  })

  it('clears waiting timers and ignores a late rejection after unmount', async () => {
    vi.useFakeTimers()
    const response = deferred()
    const view = setup(state(), vi.fn().mockReturnValue(response.promise))
    await act(async () => view.result.current.refresh())
    expect(vi.getTimerCount()).toBe(1)
    view.unmount()
    expect(vi.getTimerCount()).toBe(0)
    await act(async () => response.reject(new Error('already unmounted')))
    expect(toast.error).not.toHaveBeenCalled()
  })
})

type AllSyncProps = {
  accounts: { id: string; mailboxSourceId: string }[]
  syncStates: Record<string, AccountSyncState>
}

function setupAll(
  syncStates: Record<string, AccountSyncState> = { a: state({ accountId: 'a' }), b: state({ accountId: 'b', mailboxSourceId: 'source-b' }) },
  onRefresh = vi.fn<(accountId: string) => Promise<void>>().mockResolvedValue(undefined),
) {
  const accounts = ['a', 'b'].map((id) => ({ id, mailboxSourceId: `source-${id}` }))
  return { accounts, onRefresh, ...renderHook<ReturnType<typeof useManualSyncAll>, AllSyncProps>(
    ({ accounts, syncStates }) => useManualSyncAll(accounts, syncStates, onRefresh),
    { initialProps: { accounts, syncStates } },
  ) }
}

describe('manual all-account sync feedback', () => {
  it('keeps automatic rounds quiet and does nothing without accounts', async () => {
    const view = setupAll({ a: state({ accountId: 'a', phase: 'syncing' }) })
    expect(view.result.current.refreshing).toBe(false)
    view.rerender({ accounts: view.accounts, syncStates: { a: state({ accountId: 'a', sequence: 2 }) } })
    expect(view.result.current.refreshing).toBe(false)
    view.rerender({ accounts: [], syncStates: {} })
    await act(async () => view.result.current.refresh())
    expect(view.onRefresh).not.toHaveBeenCalled()
    expect(view.result.current.refreshing).toBe(false)
  })

  it('waits for every account to finish, rejects duplicate clicks, and stays quiet afterwards', async () => {
    const view = setupAll()
    await act(async () => {
      view.result.current.refresh()
      view.result.current.refresh()
    })
    expect(view.onRefresh.mock.calls).toEqual([['a'], ['b']])
    expect(view.result.current.refreshing).toBe(true)
    // Neither the wake-up responses nor just one completed account finishes the batch.
    view.rerender({ accounts: view.accounts, syncStates: {
      a: state({ accountId: 'a', sequence: 2 }), b: state({ accountId: 'b', mailboxSourceId: 'source-b' }),
    } })
    expect(view.result.current.refreshing).toBe(true)
    view.rerender({ accounts: view.accounts, syncStates: {
      a: state({ accountId: 'a', sequence: 2 }), b: state({ accountId: 'b', mailboxSourceId: 'source-b', phase: 'failed', sequence: 2 }),
    } })
    expect(view.result.current.refreshing).toBe(false)
    view.rerender({ accounts: view.accounts, syncStates: {
      a: state({ accountId: 'a', phase: 'syncing', sequence: 3 }),
      b: state({ accountId: 'b', mailboxSourceId: 'source-b', phase: 'syncing', sequence: 3 }),
    } })
    expect(view.result.current.refreshing).toBe(false)
  })

  it('reports a rejected request while continuing to wait for the other account', async () => {
    const onRefresh = vi.fn<(accountId: string) => Promise<void>>()
      .mockRejectedValueOnce(new Error('wake-up rejected')).mockResolvedValue(undefined)
    const view = setupAll(undefined, onRefresh)
    await act(async () => view.result.current.refresh())
    expect(view.onRefresh.mock.calls).toEqual([['a'], ['b']])
    expect(toast.error).toHaveBeenCalledTimes(1)
    expect(view.result.current.refreshing).toBe(true)
    view.rerender({ accounts: view.accounts, syncStates: {
      a: state({ accountId: 'a' }), b: state({ accountId: 'b', mailboxSourceId: 'source-b', sequence: 2 }),
    } })
    expect(view.result.current.refreshing).toBe(false)
  })

  it('joins an account already syncing without queuing another round', async () => {
    const view = setupAll({
      a: state({ accountId: 'a', phase: 'syncing' }), b: state({ accountId: 'b', mailboxSourceId: 'source-b' }),
    })
    await act(async () => view.result.current.refresh())
    expect(view.onRefresh.mock.calls).toEqual([['b']])
    view.rerender({ accounts: view.accounts, syncStates: {
      a: state({ accountId: 'a', phase: 'syncing' }), b: state({ accountId: 'b', mailboxSourceId: 'source-b', sequence: 2 }),
    } })
    expect(view.result.current.refreshing).toBe(true)
    view.rerender({ accounts: view.accounts, syncStates: {
      a: state({ accountId: 'a', sequence: 2 }), b: state({ accountId: 'b', mailboxSourceId: 'source-b', sequence: 2 }),
    } })
    expect(view.result.current.refreshing).toBe(false)
  })

  it.each(['removed', 'source replaced'] as const)('stops waiting for an account that is %s', async (change) => {
    const view = setupAll()
    await act(async () => view.result.current.refresh())
    const accounts = change === 'removed'
      ? view.accounts.filter((account) => account.id !== 'a')
      : view.accounts.map((account) => account.id === 'a' ? { ...account, mailboxSourceId: 'new-source' } : account)
    view.rerender({ accounts, syncStates: {
      a: state({ accountId: 'a' }), b: state({ accountId: 'b', mailboxSourceId: 'source-b' }),
    } })
    expect(view.result.current.refreshing).toBe(true)
    view.rerender({ accounts, syncStates: {
      a: state({ accountId: 'a' }), b: state({ accountId: 'b', mailboxSourceId: 'source-b', sequence: 2 }),
    } })
    expect(view.result.current.refreshing).toBe(false)
  })

  it('times out only accounts that never started and keeps waiting for running accounts', async () => {
    vi.useFakeTimers()
    const view = setupAll()
    await act(async () => view.result.current.refresh())
    view.rerender({ accounts: view.accounts, syncStates: {
      a: state({ accountId: 'a' }), b: state({ accountId: 'b', mailboxSourceId: 'source-b', phase: 'syncing', sequence: 2 }),
    } })
    act(() => vi.advanceTimersByTime(60_000))
    expect(view.result.current.refreshing).toBe(true)
    // A timed-out account cannot hold up the account that actually started.
    view.rerender({ accounts: view.accounts, syncStates: {
      a: state({ accountId: 'a' }), b: state({ accountId: 'b', mailboxSourceId: 'source-b', sequence: 3 }),
    } })
    expect(view.result.current.refreshing).toBe(false)
    await act(async () => view.result.current.refresh())
    expect(view.onRefresh.mock.calls).toEqual([['a'], ['b'], ['a'], ['b']])
  })
})
