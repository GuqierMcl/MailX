import { describe, expect, it } from 'vitest'
import { daySegment, lineContext, lineKey, localDay, nextBoundary } from './greeting'

const at = (hour: number, minute = 0) => new Date(2026, 10, 12, hour, minute)
const quiet = { attention: false, newMail: 0, unread: 0 }

describe('greeting', () => {
  it.each([
    [4, 'night'], [5, 'earlyMorning'], [7, 'earlyMorning'], [8, 'morning'], [10, 'morning'],
    [11, 'noon'], [12, 'noon'], [13, 'afternoon'], [16, 'afternoon'], [17, 'evening'], [20, 'evening'], [21, 'night'], [0, 'night'],
  ] as const)('%i:00 is %s', (hour, segment) => {
    expect(daySegment(at(hour))).toBe(segment)
  })

  it('wakes at the next segment boundary or midnight', () => {
    expect(nextBoundary(at(9, 30))).toEqual(at(11))
    expect(nextBoundary(at(22, 15))).toEqual(new Date(2026, 10, 13, 0, 0))
    expect(nextBoundary(at(2))).toEqual(at(5))
  })
})

describe('summary line', () => {
  // 2026-11-12 is a Thursday; 11-13 a Friday; 11-14 a Saturday; 11-16 a Monday.
  it('picks the context by priority', () => {
    const thursday = at(9)
    expect(lineContext(thursday, 'zh-CN', { ...quiet, attention: true, newMail: 3 }).kind).toBe('attention')
    expect(lineContext(new Date(2026, 1, 17, 9), 'zh-CN', { ...quiet, newMail: 3 })).toEqual({ kind: 'holiday', holiday: 'springFestival' })
    expect(lineContext(new Date(2026, 10, 7, 9), 'zh-CN', { ...quiet, newMail: 3 })).toEqual({ kind: 'solarTerm', term: 'lidong' })
    // Solar terms speak Chinese only.
    expect(lineContext(new Date(2026, 10, 7, 9), 'en', quiet).kind).toBe('weekend')
    expect(lineContext(thursday, 'zh-CN', { ...quiet, newMail: 3 })).toEqual({ kind: 'newMail', count: 3 })
    expect(lineContext(new Date(2026, 10, 16, 9), 'zh-CN', quiet).kind).toBe('monday')
    expect(lineContext(new Date(2026, 10, 13, 9), 'zh-CN', quiet).kind).toBe('friday')
    expect(lineContext(new Date(2026, 10, 14, 9), 'zh-CN', quiet).kind).toBe('weekend')
    expect(lineContext(thursday, 'zh-CN', quiet).kind).toBe('clear')
    expect(lineContext(thursday, 'zh-CN', { ...quiet, unread: 4 }).kind).toBe('steady')
  })

  it('keeps the same line all day and for any count, and may change with the context', () => {
    const morning = lineKey(at(6), { kind: 'newMail', count: 1 })
    expect(lineKey(at(23, 59), { kind: 'newMail', count: 40 })).toBe(morning)
    expect(morning).toMatch(/^now:lines\.newMail\.[abc]$/)
    expect(lineKey(at(9), { kind: 'clear' })).toMatch(/^now:lines\.clear\.[abc]$/)
    // Over a few weeks every line of a group turns up.
    const seen = new Set(Array.from({ length: 30 }, (_, day) => lineKey(new Date(2026, 0, day + 1), { kind: 'clear' })))
    expect(seen.size).toBe(3)
  })

  it('writes the local date', () => {
    expect(localDay(new Date(2026, 0, 5, 23, 30))).toBe('2026-01-05')
  })
})
