import { act, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { OverviewCode } from '@/generated/accounts'
import { LAST_VISIT_KEY, readLastVisit, useLastVisit, writeLastVisit } from './visit'
import { useFreshCodes } from './use-fresh-codes'

afterEach(() => {
  localStorage.clear()
  vi.useRealTimers()
})

describe('last visit', () => {
  it('starts at the beginning of today on first use, then reads what was saved', () => {
    const now = new Date(2026, 10, 14, 15, 30)
    expect(readLastVisit(now)).toBe(new Date(2026, 10, 14).toISOString())
    writeLastVisit(new Date(2026, 10, 14, 9))
    expect(readLastVisit(now)).toBe(new Date(2026, 10, 14, 9).toISOString())
    localStorage.setItem(LAST_VISIT_KEY, 'garbage')
    expect(readLastVisit(now)).toBe(new Date(2026, 10, 14).toISOString())
  })

  it('keeps since fixed for the visit and records the visit on hide and on leaving', () => {
    const before = new Date(Date.now() - 3_600_000).toISOString()
    localStorage.setItem(LAST_VISIT_KEY, before)
    const { result, unmount } = renderHook(() => useLastVisit())
    expect(result.current).toBe(before)

    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    act(() => { document.dispatchEvent(new Event('visibilitychange')) })
    visibility.mockRestore()
    const hidden = localStorage.getItem(LAST_VISIT_KEY)!
    expect(Date.parse(hidden)).toBeGreaterThan(Date.parse(before))
    expect(result.current).toBe(before)

    localStorage.setItem(LAST_VISIT_KEY, before)
    unmount()
    expect(Date.parse(localStorage.getItem(LAST_VISIT_KEY)!)).toBeGreaterThan(Date.parse(before))
  })
})

describe('verification code expiry', () => {
  const code = (id: string, receivedAt: Date): OverviewCode => ({
    emailId: id, accountId: 'a', folderId: 'f', from: { name: null, address: 'x@example.com' }, code: '123456', receivedAt: receivedAt.toISOString(),
  })

  it('drops each code fifteen minutes after it arrived, without polling', () => {
    vi.useFakeTimers({ now: new Date(2026, 10, 14, 10, 0) })
    const codes = [code('new', new Date(2026, 10, 14, 9, 58)), code('old', new Date(2026, 10, 14, 9, 50))]
    const { result } = renderHook(() => useFreshCodes(codes))
    expect(result.current.map((item) => item.emailId)).toEqual(['new', 'old'])
    act(() => { vi.advanceTimersByTime(5 * 60_000 + 100) })
    expect(result.current.map((item) => item.emailId)).toEqual(['new'])
    act(() => { vi.advanceTimersByTime(8 * 60_000) })
    expect(result.current).toEqual([])
    expect(vi.getTimerCount()).toBe(0)
  })
})
