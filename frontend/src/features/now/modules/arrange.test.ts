import { describe, expect, it } from 'vitest'
import type { NowSettings } from '@/generated/accounts'
import { arrange, fixedOrder, orderModules, suppressed } from './arrange'
import { NOW_MODULES } from './registry'
import type { NowContext, NowData, NowModule, NowModuleId } from './types'

const EMPTY: NowData = {
  attention: [], draftCount: 0, codes: [], inbox: { newMail: 0, unread: 0, starredUnread: 0, rows: [] }, attachments: [], activity: undefined,
  away: { sinceTime: 0, ms: 0, total: 0, senders: [] }, unreadSenders: [],
}

/** Thursday 2026-11-26, 09:00 local. */
const MORNING = new Date(2026, 10, 26, 9)

function context(data: Partial<NowData> = {}, extra: Partial<Omit<NowContext, 'data'>> = {}): NowContext {
  return { now: MORNING, awayMs: 0, firstToday: false, ...extra, data: { ...EMPTY, ...data } }
}

const CODE = { emailId: 'e1' } as NowData['codes'][number]
const ATTENTION = { id: 'failed' } as NowData['attention'][number]
const SENDER = { address: 'shop@example.test', name: 'Shop', count: 4 }
const TODAY = '2026-11-26'
const AUTO: NowSettings = { arrangement: 'auto', order: [], hidden: [] }
const DEFAULT_ORDER = ['attention', 'codes', 'drafts', 'away', 'inbox', 'cleanup', 'activity', 'attachments']

const by = (id: string) => NOW_MODULES.find((module) => module.id === id)!
const score = (id: string, ctx: NowContext = context()) => by(id).score(ctx)
/** Some mail, so the inbox is not quiet. */
const BUSY = { inbox: { newMail: 3, unread: 5, starredUnread: 0, rows: [] } }

/** A module for the ordering rules alone. */
function fake(id: string, zone: NowModule['zone'], score = 0, hideable = true): NowModule {
  return { id: id as NowModuleId, zone, hideable, title: () => id, score: () => score, signature: () => `sig-${id}`, hasContent: () => true, render: () => null }
}

describe('scores (the table in now-page.md 4.3)', () => {
  it('codes: 320 with a code, else the drafts level', () => {
    expect(score('codes', context({ codes: [CODE] }))).toBe(320)
    expect(score('codes')).toBe(300)
  })

  it('drafts: 300', () => {
    expect(score('drafts')).toBe(300)
  })

  it('inbox: 295 from 20 new messages, else 260', () => {
    const inbox = (newMail: number) => context({ inbox: { newMail, unread: 0, starredUnread: 0, rows: [] } })
    expect(score('inbox', inbox(19))).toBe(260)
    expect(score('inbox', inbox(20))).toBe(295)
    expect(score('inbox', inbox(0))).toBe(260)
  })

  it('away: 290', () => {
    expect(score('away')).toBe(290)
  })

  it('cleanup: 240', () => {
    expect(score('cleanup')).toBe(240)
  })

  it('attachments: 90', () => {
    expect(score('attachments')).toBe(90)
  })

  it('activity: 100 on a busy weekday morning', () => {
    expect(score('activity', context(BUSY))).toBe(100)
  })

  it('activity: 230 when the inbox has no new mail and nothing unread', () => {
    expect(score('activity', context())).toBe(230)
    expect(score('activity', context({ inbox: { newMail: 0, unread: 2, starredUnread: 0, rows: [] } }))).toBe(100)
    expect(score('activity', context({ inbox: { newMail: 2, unread: 0, starredUnread: 0, rows: [] } }))).toBe(100)
  })

  it('activity: 230 on the first visit of a Monday, but not on a later one or another day', () => {
    const monday = new Date(2026, 10, 23, 9)
    expect(score('activity', context(BUSY, { now: monday, firstToday: true }))).toBe(230)
    expect(score('activity', context(BUSY, { now: monday, firstToday: false }))).toBe(100)
    expect(score('activity', context(BUSY, { now: MORNING, firstToday: true }))).toBe(100)
  })

  it('activity: 230 from 18:00 local on', () => {
    expect(score('activity', context(BUSY, { now: new Date(2026, 10, 26, 17, 59) }))).toBe(100)
    expect(score('activity', context(BUSY, { now: new Date(2026, 10, 26, 18, 0) }))).toBe(230)
    expect(score('activity', context(BUSY, { now: new Date(2026, 10, 26, 23, 30) }))).toBe(230)
  })
})

describe('automatic order', () => {
  it('sorts by score alone, then keeps the registry order of equal scores', () => {
    const modules = [fake('a', 'review', 5), fake('b', 'act', 5), fake('c', 'flow', 9), fake('d', 'act', 1)]
    expect(orderModules(modules, context(), AUTO)).toEqual(['c', 'a', 'b', 'd'])
  })

  it('keeps attention first whatever the scores say', () => {
    const modules = [fake('a', 'act', 999), fake('attention', 'act', 0, false)]
    expect(orderModules(modules, context(), AUTO)).toEqual(['attention', 'a'])
  })

  it('orders a quiet morning: act, away, inbox, cleanup, then activity and attachments', () => {
    expect(orderModules(NOW_MODULES, context(), AUTO)).toEqual(DEFAULT_ORDER)
  })

  it('leads with attention, and with codes before drafts while a code is there', () => {
    const order = orderModules(NOW_MODULES, context({ codes: [CODE], draftCount: 3, attention: [ATTENTION] }), AUTO)
    expect(order.slice(0, 3)).toEqual(['attention', 'codes', 'drafts'])
  })

  it('puts the inbox ahead of away when 20 or more messages are new', () => {
    const order = orderModules(NOW_MODULES, context({ inbox: { newMail: 25, unread: 25, starredUnread: 0, rows: [] } }), AUTO)
    expect(order.indexOf('inbox')).toBeLessThan(order.indexOf('away'))
    expect(order.indexOf('inbox')).toBeGreaterThan(order.indexOf('drafts'))
  })

  it('puts a boosted activity after inbox and cleanup but before attachments', () => {
    const order = orderModules(NOW_MODULES, context(BUSY, { now: new Date(2026, 10, 26, 20) }), AUTO)
    expect(order).toEqual(['attention', 'codes', 'drafts', 'away', 'inbox', 'cleanup', 'activity', 'attachments'])
    // Without the boost it drops below, with attachments still last.
    expect(orderModules(NOW_MODULES, context(BUSY), AUTO).slice(-2)).toEqual(['activity', 'attachments'])
  })

  it('orders empty modules too, so a module that gains content later has its slot', () => {
    expect(orderModules(NOW_MODULES, context(), AUTO)).toHaveLength(NOW_MODULES.length)
  })

  it('is the same for the same inputs', () => {
    const ctx = context({ codes: [CODE], inbox: { newMail: 30, unread: 9, starredUnread: 1, rows: [] } }, { now: new Date(2026, 10, 23, 19), firstToday: true })
    expect(orderModules(NOW_MODULES, ctx, AUTO)).toEqual(orderModules(NOW_MODULES, structuredClone(ctx), AUTO))
  })

  it('never ranks a flow module above codes or drafts that have content, whatever the situation', () => {
    const hours = [0, 9, 18, 23]
    const days = [23, 24, 26, 28]
    for (const hour of hours) {
      for (const day of days) {
        for (const newMail of [0, 5, 20, 500]) {
          const ctx = context({ codes: [CODE], draftCount: 2, inbox: { newMail, unread: newMail, starredUnread: 0, rows: [] }, unreadSenders: [SENDER] }, { now: new Date(2026, 10, day, hour), firstToday: true, awayMs: 99 * 3_600_000 })
          const order = orderModules(NOW_MODULES, ctx, AUTO)
          for (const id of ['away', 'inbox', 'cleanup', 'activity', 'attachments'] as NowModuleId[]) {
            expect(order.indexOf(id)).toBeGreaterThan(order.indexOf('codes'))
            expect(order.indexOf(id)).toBeGreaterThan(order.indexOf('drafts'))
          }
        }
      }
    }
  })

  it('never lets activity beat the inbox, and keeps attachments last', () => {
    for (const hour of [0, 9, 18, 23]) {
      for (const newMail of [0, 1, 20]) {
        for (const unread of [0, 4]) {
          const ctx = context({ inbox: { newMail, unread, starredUnread: 0, rows: [] } }, { now: new Date(2026, 10, 23, hour), firstToday: true })
          const order = orderModules(NOW_MODULES, ctx, AUTO)
          expect(order.indexOf('activity')).toBeGreaterThan(order.indexOf('inbox'))
          expect(order[order.length - 1]).toBe('attachments')
        }
      }
    }
  })
})

describe('fixed order', () => {
  const fixed = (order: string[]) => ({ arrangement: 'fixed' as const, order })

  it('follows the user order with attention first, whatever the order says', () => {
    expect(orderModules(NOW_MODULES, context(), fixed(['activity', 'inbox', 'attention', 'drafts', 'codes', 'attachments'])))
      .toEqual(['attention', 'activity', 'inbox', 'drafts', 'codes', 'attachments', 'away', 'cleanup'])
  })

  it('appends modules missing from the order in the default order', () => {
    expect(orderModules(NOW_MODULES, context(), fixed(['activity', 'drafts'])))
      .toEqual(['attention', 'activity', 'drafts', 'codes', 'away', 'inbox', 'cleanup', 'attachments'])
  })

  it('ignores unknown ids and repeats, and an empty order is the default order', () => {
    expect(orderModules(NOW_MODULES, context(), fixed(['future', 'inbox', 'inbox', 'x'])))
      .toEqual(['attention', 'inbox', 'codes', 'drafts', 'away', 'cleanup', 'activity', 'attachments'])
    expect(orderModules(NOW_MODULES, context(), fixed([]))).toEqual(DEFAULT_ORDER)
  })

  it('does not depend on the context', () => {
    const order = ['activity', 'drafts']
    expect(orderModules(NOW_MODULES, context({ codes: [CODE] }), fixed(order))).toEqual(orderModules(NOW_MODULES, context(), fixed(order)))
  })

  it('lists the ids a registry knows, without the pinned one', () => {
    expect(fixedOrder(['attention', 'a', 'b', 'c'], ['c', 'attention', 'zzz'])).toEqual(['c', 'a', 'b'])
  })
})

describe('hiding', () => {
  const [attention, codes] = NOW_MODULES

  it('removes modules the user hid for good, in either arrangement', () => {
    expect(arrange(NOW_MODULES, context(), { ...AUTO, hidden: ['codes', 'activity'] }, {}, TODAY)).toEqual(['attention', 'drafts', 'away', 'inbox', 'cleanup', 'attachments'])
    expect(arrange(NOW_MODULES, context(), { arrangement: 'fixed', order: ['activity', 'codes'], hidden: ['codes'] }, {}, TODAY))
      .toEqual(['attention', 'activity', 'drafts', 'away', 'inbox', 'cleanup', 'attachments'])
  })

  it('never removes a module that cannot be hidden', () => {
    const snoozed = { attention: { day: TODAY, signature: attention.signature(EMPTY) } }
    expect(arrange(NOW_MODULES, context(), { ...AUTO, hidden: ['attention'] }, snoozed, TODAY)).toContain('attention')
    expect(suppressed(attention, { hidden: ['attention'] }, snoozed, EMPTY, TODAY)).toBe(false)
  })

  it('keeps hidden ids it does not know without effect', () => {
    expect(arrange(NOW_MODULES, context(), { ...AUTO, hidden: ['future'] }, {}, TODAY)).toHaveLength(NOW_MODULES.length)
  })

  it('hides a module for today while its content is unchanged', () => {
    const data = { ...EMPTY, codes: [CODE] }
    const snoozed = { codes: { day: TODAY, signature: codes.signature(data) } }
    expect(suppressed(codes, { hidden: [] }, snoozed, data, TODAY)).toBe(true)
    expect(arrange(NOW_MODULES, context({ codes: [CODE] }), AUTO, snoozed, TODAY)).not.toContain('codes')
  })

  it('brings it back when the content changes, and on the next day', () => {
    const data = { ...EMPTY, codes: [CODE] }
    const snoozed = { codes: { day: TODAY, signature: codes.signature(data) } }
    const more = { ...EMPTY, codes: [CODE, { emailId: 'e2' } as NowData['codes'][number]] }
    expect(suppressed(codes, { hidden: [] }, snoozed, more, TODAY)).toBe(false)
    expect(suppressed(codes, { hidden: [] }, snoozed, data, '2026-11-27')).toBe(false)
  })

  it('does not move the others when one is snoozed', () => {
    const all = orderModules(NOW_MODULES, context(), AUTO)
    const snoozed = { inbox: { day: TODAY, signature: by('inbox').signature(EMPTY) } }
    expect(arrange(NOW_MODULES, context(), AUTO, snoozed, TODAY)).toEqual(all.filter((id) => id !== 'inbox'))
  })
})

describe('signatures', () => {
  it('change with the content a person would notice', () => {
    expect(by('drafts').signature({ ...EMPTY, draftCount: 1 })).not.toBe(by('drafts').signature({ ...EMPTY, draftCount: 2 }))
    expect(by('inbox').signature({ ...EMPTY, inbox: { ...EMPTY.inbox, unread: 3 } })).not.toBe(by('inbox').signature({ ...EMPTY, inbox: { ...EMPTY.inbox, unread: 4 } }))
    expect(by('codes').signature({ ...EMPTY, codes: [CODE] })).not.toBe(by('codes').signature(EMPTY))
    expect(by('activity').signature(EMPTY)).toBe('')
  })

  it('of away is the last visit and the new mail total; of cleanup the senders and their counts', () => {
    const away = (sinceTime: number, total: number) => ({ ...EMPTY, away: { sinceTime, ms: 0, total, senders: [] } })
    expect(by('away').signature(away(1, 5))).toBe('1|5')
    expect(by('away').signature(away(1, 5))).not.toBe(by('away').signature(away(1, 6)))
    expect(by('away').signature(away(1, 5))).not.toBe(by('away').signature(away(2, 5)))
    const cleanup = (count: number) => ({ ...EMPTY, unreadSenders: [{ ...SENDER, count }] })
    expect(by('cleanup').signature(cleanup(4))).toBe('shop@example.test:4')
    expect(by('cleanup').signature(cleanup(4))).not.toBe(by('cleanup').signature(cleanup(5)))
  })
})

describe('what the new modules need to show', () => {
  const HOUR = 3_600_000
  const away = (ms: number, total: number) => ({ ...EMPTY, away: { sinceTime: 0, ms, total, senders: [SENDER] } })

  it('away shows after three hours with new mail, and not before or without', () => {
    expect(by('away').hasContent(away(3 * HOUR, 1))).toBe(true)
    expect(by('away').hasContent(away(3 * HOUR - 1, 12))).toBe(false)
    expect(by('away').hasContent(away(30 * HOUR, 0))).toBe(false)
  })

  it('cleanup shows when some sender has piled up unread mail', () => {
    expect(by('cleanup').hasContent({ ...EMPTY, unreadSenders: [SENDER] })).toBe(true)
    expect(by('cleanup').hasContent(EMPTY)).toBe(false)
  })

  it('both can be hidden and snoozed', () => {
    expect(by('away').hideable).toBe(true)
    expect(by('cleanup').hideable).toBe(true)
    const data = { ...EMPTY, unreadSenders: [SENDER] }
    const snoozed = { cleanup: { day: TODAY, signature: by('cleanup').signature(data) } }
    expect(suppressed(by('cleanup'), { hidden: [] }, snoozed, data, TODAY)).toBe(true)
    expect(suppressed(by('cleanup'), { hidden: ['cleanup'] }, {}, data, TODAY)).toBe(true)
  })
})
