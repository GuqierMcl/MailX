import { act, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, exampleFolderSyncState, exampleSyncState } from '@/features/accounts/test-fixtures'
import { compareWeeks, dayStarts, syncSummary } from './activity'
import { useMinuteClock } from './visit'

describe('day starts', () => {
  it('are fourteen ascending local midnights ending with today', () => {
    const now = new Date(2026, 10, 14, 15, 30)
    const starts = dayStarts(now).map((start) => new Date(start))
    expect(starts).toHaveLength(14)
    expect(starts[13]).toEqual(new Date(2026, 10, 14))
    expect(starts[0]).toEqual(new Date(2026, 10, 1))
    for (const [index, start] of starts.entries()) {
      expect([start.getHours(), start.getMinutes()]).toEqual([0, 0])
      if (index > 0) expect(start.getTime()).toBeGreaterThan(starts[index - 1].getTime())
    }
  })

  it('crosses the turn of a month and a year', () => {
    const starts = dayStarts(new Date(2027, 0, 3, 8)).map((start) => new Date(start))
    expect(starts[0]).toEqual(new Date(2026, 11, 21))
    expect(starts[13]).toEqual(new Date(2027, 0, 3))
  })

  describe('in a zone with daylight saving time', () => {
    // Node reads TZ again whenever it is assigned.
    const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env
    const original = env.TZ
    afterEach(() => {
      if (original === undefined) delete env.TZ
      else env.TZ = original
    })

    it('keeps each start at local midnight across the change', () => {
      env.TZ = 'America/New_York'
      // Only meaningful when the runtime honors the zone change.
      if (new Date(2026, 6, 1).getTimezoneOffset() !== 240) return
      // Clocks went back on 1 November 2026, so that day lasted 25 hours.
      const starts = dayStarts(new Date(2026, 10, 14, 12)).map((start) => new Date(start))
      expect(starts.map((start) => start.getHours())).toEqual(Array(14).fill(0))
      expect(starts.map((start) => start.getDate())).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14])
      expect(starts[1].getTime() - starts[0].getTime()).toBe(25 * 3_600_000)
      expect(starts[0].toISOString()).toBe('2026-11-01T04:00:00.000Z')
      expect(starts[13].toISOString()).toBe('2026-11-14T05:00:00.000Z')
    })
  })
})

describe('week comparison', () => {
  it('only states the count when the earlier week is not fully cached', () => {
    expect(compareWeeks(86, 10, false)).toEqual({ kind: 'only' })
  })

  it('calls a change of two messages or a tenth the same', () => {
    expect(compareWeeks(12, 10, true)).toEqual({ kind: 'same' })
    expect(compareWeeks(8, 10, true)).toEqual({ kind: 'same' })
    expect(compareWeeks(86, 80, true)).toEqual({ kind: 'same' })
    expect(compareWeeks(0, 0, true)).toEqual({ kind: 'same' })
  })

  it('gives the difference otherwise', () => {
    expect(compareWeeks(86, 74, true)).toEqual({ kind: 'more', diff: 12 })
    expect(compareWeeks(5, 9, true)).toEqual({ kind: 'fewer', diff: 4 })
  })
})

describe('sync summary', () => {
  const first = exampleAccount({ id: 'a' })
  const second = exampleAccount({ id: 'b' })

  it('is nothing without accounts or before every account reported', () => {
    expect(syncSummary([], {})).toBeNull()
    expect(syncSummary([first, second], { a: exampleSyncState({ accountId: 'a' }) })).toBeNull()
  })

  it('gives the oldest last sync among accounts', () => {
    expect(syncSummary([first, second], {
      a: exampleSyncState({ accountId: 'a', lastSyncedAt: '2026-11-14T08:05:00.000Z' }),
      b: exampleSyncState({ accountId: 'b', lastSyncedAt: '2026-11-14T08:01:00.000Z' }),
    })).toEqual({ kind: 'synced', at: '2026-11-14T08:01:00.000Z' })
  })

  it('falls back to the folders when the account has not finished a round this session', () => {
    expect(syncSummary([first], {
      a: exampleSyncState({ accountId: 'a', lastSyncedAt: null, folders: [
        exampleFolderSyncState('inbox', { lastSyncedAt: '2026-11-14T08:03:00.000Z' }),
        exampleFolderSyncState('sent', { lastSyncedAt: '2026-11-14T08:02:00.000Z' }),
      ] }),
    })).toEqual({ kind: 'synced', at: '2026-11-14T08:02:00.000Z' })
  })

  it('says syncing while a round runs or before the first one finished', () => {
    expect(syncSummary([first, second], {
      a: exampleSyncState({ accountId: 'a' }),
      b: exampleSyncState({ accountId: 'b', phase: 'syncing' }),
    })).toEqual({ kind: 'syncing' })
    expect(syncSummary([first], { a: exampleSyncState({ accountId: 'a', lastSyncedAt: null }) })).toEqual({ kind: 'syncing' })
  })

  it('reports a failure first', () => {
    expect(syncSummary([first, second], {
      a: exampleSyncState({ accountId: 'a', phase: 'syncing' }),
      b: exampleSyncState({ accountId: 'b', failure: { code: 'network', message: '', detail: null, willRetry: true }, failureSince: '2026-10-18T08:00:00.000Z' }),
    })).toEqual({ kind: 'issue' })
  })
})

describe('minute clock', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('ticks once a minute while visible, stops while hidden and ticks on return', () => {
    vi.useFakeTimers({ now: new Date(2026, 10, 14, 9, 0, 0) })
    const { result } = renderHook(() => useMinuteClock())
    const start = result.current
    act(() => { vi.advanceTimersByTime(59_000) })
    expect(result.current).toBe(start)
    act(() => { vi.advanceTimersByTime(1_000) })
    expect(result.current).toBe(start + 60_000)

    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    act(() => { document.dispatchEvent(new Event('visibilitychange')) })
    act(() => { vi.advanceTimersByTime(5 * 60_000) })
    expect(result.current).toBe(start + 60_000)

    visibility.mockReturnValue('visible')
    act(() => { document.dispatchEvent(new Event('visibilitychange')) })
    expect(result.current).toBe(start + 6 * 60_000)
    act(() => { vi.advanceTimersByTime(60_000) })
    expect(result.current).toBe(start + 7 * 60_000)
  })
})
