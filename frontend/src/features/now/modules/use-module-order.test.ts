import { renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { NowSettings } from '@/generated/accounts'
import { useModuleOrder } from './use-module-order'
import type { NowContext } from './types'

const context = (codes: number): NowContext => ({
  now: new Date(2026, 10, 26, 9), awayMs: 0, firstToday: false,
  data: { attention: [], draftCount: 0, codes: Array.from({ length: codes }, (_, i) => ({ emailId: `e${i}` }) as NowContext['data']['codes'][number]), inbox: { newMail: 0, unread: 0, starredUnread: 0, rows: [] }, attachments: [], activity: undefined },
})
const auto: NowSettings = { arrangement: 'auto', order: [], hidden: [] }

describe('useModuleOrder', () => {
  it('waits for the page, then works the order out once for the stay', () => {
    const { result, rerender } = renderHook(({ ready, settings, ctx }) => useModuleOrder(ready, settings, ctx), { initialProps: { ready: false, settings: auto, ctx: context(0) } })
    expect(result.current).toBeNull()
    rerender({ ready: true, settings: auto, ctx: context(0) })
    const first = result.current
    expect(first).toEqual(['attention', 'codes', 'drafts', 'inbox', 'attachments', 'activity'])
    // New data does not order again.
    rerender({ ready: true, settings: auto, ctx: context(2) })
    expect(result.current).toBe(first)
    // Hiding is not part of the order; a change of the arrangement is.
    rerender({ ready: true, settings: { ...auto, hidden: ['codes'] }, ctx: context(2) })
    expect(result.current).toBe(first)
    rerender({ ready: true, settings: { arrangement: 'fixed', order: ['activity'], hidden: [] }, ctx: context(2) })
    expect(result.current).toEqual(['attention', 'activity', 'codes', 'drafts', 'inbox', 'attachments'])
  })
})
