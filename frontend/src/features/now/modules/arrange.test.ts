import { describe, expect, it } from 'vitest'
import type { NowSettings } from '@/generated/accounts'
import { arrange, fixedOrder, orderModules, suppressed } from './arrange'
import { NOW_MODULES } from './registry'
import type { NowContext, NowData, NowModule, NowModuleId } from './types'

const EMPTY: NowData = { attention: [], draftCount: 0, codes: [], inbox: { newMail: 0, unread: 0, starredUnread: 0, rows: [] }, attachments: [], activity: undefined }

function context(data: Partial<NowData> = {}): NowContext {
  return { now: new Date(2026, 10, 26, 9), awayMs: 0, firstToday: false, data: { ...EMPTY, ...data } }
}

const CODE = { emailId: 'e1' } as NowData['codes'][number]
const ATTENTION = { id: 'failed' } as NowData['attention'][number]
const TODAY = '2026-11-26'
const AUTO: NowSettings = { arrangement: 'auto', order: [], hidden: [] }

/** A module for the ordering rules alone. */
function fake(id: string, zone: NowModule['zone'], score = 0, hideable = true): NowModule {
  return { id: id as NowModuleId, zone, hideable, title: () => id, score: () => score, signature: () => `sig-${id}`, hasContent: () => true, render: () => null }
}

describe('automatic order', () => {
  it('lists the zones in their fixed order, then keeps the registry order of equal scores', () => {
    const modules = [fake('review1', 'review'), fake('flow1', 'flow'), fake('act1', 'act'), fake('flow2', 'flow'), fake('act2', 'act')]
    expect(orderModules(modules, context(), AUTO)).toEqual(['act1', 'act2', 'flow1', 'flow2', 'review1'])
  })

  it('puts a higher score first inside its zone and never across zones', () => {
    const modules = [fake('a', 'act', 1), fake('b', 'act', 5), fake('c', 'flow', 99), fake('d', 'act', 5)]
    expect(orderModules(modules, context(), AUTO)).toEqual(['b', 'd', 'a', 'c'])
  })

  it('keeps the page as it was: attention, codes, drafts, inbox, attachments, activity', () => {
    expect(orderModules(NOW_MODULES, context(), AUTO)).toEqual(['attention', 'codes', 'drafts', 'inbox', 'attachments', 'activity'])
  })

  it('leads the act zone with attention, and with codes before drafts while a code is there', () => {
    const order = orderModules(NOW_MODULES, context({ codes: [CODE], draftCount: 3, attention: [ATTENTION] }), AUTO)
    expect(order.slice(0, 3)).toEqual(['attention', 'codes', 'drafts'])
  })

  it('orders empty modules too, so a module that gains content later has its slot', () => {
    expect(orderModules(NOW_MODULES, context(), AUTO)).toHaveLength(NOW_MODULES.length)
  })
})

describe('fixed order', () => {
  const fixed = (order: string[]) => ({ arrangement: 'fixed' as const, order })

  it('follows the user order with attention first, whatever the order says', () => {
    expect(orderModules(NOW_MODULES, context(), fixed(['activity', 'inbox', 'attention', 'drafts', 'codes', 'attachments'])))
      .toEqual(['attention', 'activity', 'inbox', 'drafts', 'codes', 'attachments'])
  })

  it('appends modules missing from the order in the default order', () => {
    expect(orderModules(NOW_MODULES, context(), fixed(['activity', 'drafts'])))
      .toEqual(['attention', 'activity', 'drafts', 'codes', 'inbox', 'attachments'])
  })

  it('ignores unknown ids and repeats, and an empty order is the default order', () => {
    expect(orderModules(NOW_MODULES, context(), fixed(['future', 'inbox', 'inbox', 'x'])))
      .toEqual(['attention', 'inbox', 'codes', 'drafts', 'attachments', 'activity'])
    expect(orderModules(NOW_MODULES, context(), fixed([]))).toEqual(['attention', 'codes', 'drafts', 'inbox', 'attachments', 'activity'])
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
    expect(arrange(NOW_MODULES, context(), { ...AUTO, hidden: ['codes', 'activity'] }, {}, TODAY)).toEqual(['attention', 'drafts', 'inbox', 'attachments'])
    expect(arrange(NOW_MODULES, context(), { arrangement: 'fixed', order: ['activity', 'codes'], hidden: ['codes'] }, {}, TODAY))
      .toEqual(['attention', 'activity', 'drafts', 'inbox', 'attachments'])
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
    const snoozed = { inbox: { day: TODAY, signature: NOW_MODULES[3].signature(EMPTY) } }
    expect(arrange(NOW_MODULES, context(), AUTO, snoozed, TODAY)).toEqual(all.filter((id) => id !== 'inbox'))
  })
})

describe('signatures', () => {
  it('change with the content a person would notice', () => {
    const by = (id: string) => NOW_MODULES.find((module) => module.id === id)!
    expect(by('drafts').signature({ ...EMPTY, draftCount: 1 })).not.toBe(by('drafts').signature({ ...EMPTY, draftCount: 2 }))
    expect(by('inbox').signature({ ...EMPTY, inbox: { ...EMPTY.inbox, unread: 3 } })).not.toBe(by('inbox').signature({ ...EMPTY, inbox: { ...EMPTY.inbox, unread: 4 } }))
    expect(by('codes').signature({ ...EMPTY, codes: [CODE] })).not.toBe(by('codes').signature(EMPTY))
    expect(by('activity').signature(EMPTY)).toBe('')
  })
})
