import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useDelayedFlag } from './use-delayed-flag'

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

describe('useDelayedFlag', () => {
  it('never turns on for a load that finishes within the delay', () => {
    const { result, rerender } = renderHook(({ active }) => useDelayedFlag(active, 150, 300), { initialProps: { active: true } })
    act(() => { vi.advanceTimersByTime(100) })
    rerender({ active: false })
    act(() => { vi.advanceTimersByTime(1000) })
    expect(result.current).toBe(false)
  })

  it('turns on after the delay and stays on for the minimum time', () => {
    const { result, rerender } = renderHook(({ active }) => useDelayedFlag(active, 150, 300), { initialProps: { active: true } })
    act(() => { vi.advanceTimersByTime(150) })
    expect(result.current).toBe(true)
    act(() => { vi.advanceTimersByTime(50) })
    rerender({ active: false })
    act(() => { vi.advanceTimersByTime(200) })
    expect(result.current).toBe(true)
    act(() => { vi.advanceTimersByTime(60) })
    expect(result.current).toBe(false)
  })
})
