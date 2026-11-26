import { describe, expect, it, vi } from 'vitest'
import type { WeekStartsOn } from '@/generated/accounts'
import { dateGroupBoundaries, emailDateGroup, formatGroupedListTime } from './date-groups'

describe('mail list date groups', () => {
  // Monday, with Sunday reserved for Yesterday even when it is in the previous week.
  const monday = new Date(2026, 10, 23, 12)
  const received = (day: number, hour = 0, minute = 0) => new Date(2026, 10, day, hour, minute).toISOString()

  it('prioritizes today and yesterday, then honors the selected natural week', () => {
    const mondayStart = dateGroupBoundaries(monday, 'monday')
    const saturdayStart = dateGroupBoundaries(monday, 'saturday')
    expect(emailDateGroup(received(23), mondayStart)).toBe('today')
    expect(emailDateGroup(received(22), mondayStart)).toBe('yesterday')
    expect(emailDateGroup(received(21), mondayStart)).toBe('lastWeek')
    expect(emailDateGroup(received(21), saturdayStart)).toBe('thisWeek')
    expect(emailDateGroup(received(16), mondayStart)).toBe('lastWeek')
    expect(emailDateGroup(received(15, 23, 59), mondayStart)).toBe('earlier')
  })

  it.each<[WeekStartsOn, number]>([
    ['sunday', 22], ['monday', 23], ['tuesday', 17], ['wednesday', 18],
    ['thursday', 19], ['friday', 20], ['saturday', 21],
  ])('uses local midnight for a %s week start', (weekStartsOn, expectedDay) => {
    const bounds = dateGroupBoundaries(monday, weekStartsOn)
    expect(bounds.thisWeek).toBe(Date.parse(received(expectedDay)))
    expect(bounds.lastWeek).toBe(Date.parse(received(expectedDay - 7)))
  })

  it('regroups at local midnight across the change of year', () => {
    const newYear = new Date(2027, 0, 1)
    const before = dateGroupBoundaries(new Date(newYear.getTime() - 1), 'sunday')
    const after = dateGroupBoundaries(newYear, 'sunday')
    const timestamp = new Date(newYear.getTime() - 1).toISOString()
    expect(before.tomorrow).toBe(newYear.getTime())
    expect(emailDateGroup(timestamp, before)).toBe('today')
    expect(emailDateGroup(timestamp, after)).toBe('yesterday')
    expect(emailDateGroup(newYear.toISOString(), before)).toBe('future')
  })

  it('keeps local day boundaries across a daylight saving transition', () => {
    vi.stubEnv('TZ', 'America/New_York')
    try {
      const bounds = dateGroupBoundaries(new Date(2026, 10, 2, 12), 'monday')
      expect(bounds.today - bounds.yesterday).toBe(25 * 3_600_000)
      expect(emailDateGroup('2026-11-02T04:59:59.999Z', bounds)).toBe('yesterday')
      expect(emailDateGroup('2026-11-02T05:00:00.000Z', bounds)).toBe('today')
    } finally {
      vi.unstubAllEnvs()
    }
  })

  it('shows times for today and yesterday and localized short dates for the other groups', () => {
    const bounds = dateGroupBoundaries(monday, 'monday')
    expect(formatGroupedListTime(received(23, 9, 5), 'today', bounds, 'zh-CN')).toBe('09:05')
    expect(formatGroupedListTime(received(22, 14, 30), 'yesterday', bounds, 'en')).toBe('14:30')
    expect(formatGroupedListTime(received(21), 'lastWeek', bounds, 'en')).toBe('11/21')
    expect(formatGroupedListTime(new Date(2025, 10, 21).toISOString(), 'earlier', bounds, 'en')).toBe('11/21/2025')
    expect(emailDateGroup('invalid', bounds)).toBe('earlier')
    expect(formatGroupedListTime('invalid', 'earlier', bounds, 'en')).toBe('')
  })
})
