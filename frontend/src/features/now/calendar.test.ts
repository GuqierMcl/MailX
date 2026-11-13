import { describe, expect, it } from 'vitest'
import { holidayOn, lunarDate, solarTermDate, solarTermOn } from './calendar'

describe('solar terms', () => {
  it.each([
    [2000, 'xiaohan', 1, 6],
    [2019, 'xiaohan', 1, 5],
    [2024, 'lichun', 2, 4],
    [2025, 'lichun', 2, 3],
    [2026, 'yushui', 2, 18],
    [2024, 'qingming', 4, 4],
    [2023, 'xiazhi', 6, 21],
    [2026, 'lidong', 11, 7],
    [2021, 'dongzhi', 12, 21],
    [2025, 'dongzhi', 12, 21],
  ] as const)('%i %s falls on %i/%i', (year, term, month, day) => {
    expect(solarTermDate(year, term)).toEqual({ month, day })
  })

  it('finds the term on its local date and nothing on other days', () => {
    expect(solarTermOn(new Date(2024, 1, 4))).toBe('lichun')
    expect(solarTermOn(new Date(2026, 10, 7))).toBe('lidong')
    expect(solarTermOn(new Date(2026, 10, 14))).toBeNull()
  })

  it('gives nothing outside 2000-2099', () => {
    expect(solarTermDate(1999, 'dongzhi')).toBeNull()
    expect(solarTermOn(new Date(2100, 11, 21))).toBeNull()
  })
})

describe('holidays', () => {
  it('reads the Chinese calendar in this runtime', () => {
    expect(lunarDate(new Date(2026, 1, 17))).toEqual({ month: 1, day: 1, leap: false })
  })

  it.each([
    [new Date(2026, 1, 17), 'springFestival'],
    [new Date(2026, 2, 3), 'lantern'],
    [new Date(2025, 4, 31), 'dragonBoat'],
    [new Date(2025, 7, 29), 'qixi'],
    [new Date(2025, 9, 6), 'midAutumn'],
    [new Date(2027, 0, 1), 'newYear'],
  ] as const)('%s is %s in Chinese', (date, holiday) => {
    expect(holidayOn(date, 'zh-CN')).toBe(holiday)
  })

  it('skips the same day of a leap month', () => {
    // 2009 had a leap fifth month; its fifth day is not the Dragon Boat Festival.
    expect(lunarDate(new Date(2009, 5, 27))).toMatchObject({ month: 5, day: 5, leap: true })
    expect(holidayOn(new Date(2009, 5, 27), 'zh-CN')).toBeNull()
  })

  it('keeps English to New Year’s Day', () => {
    expect(holidayOn(new Date(2026, 1, 17), 'en')).toBeNull()
    expect(holidayOn(new Date(2026, 0, 1), 'en')).toBe('newYear')
    expect(holidayOn(new Date(2026, 10, 14), 'zh-CN')).toBeNull()
  })
})
