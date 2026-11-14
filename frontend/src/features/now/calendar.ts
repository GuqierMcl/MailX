/**
 * Holidays and the 24 solar terms for the Now page's preset lines. Everything is
 * computed on this device from the local date; nothing is fetched.
 */

export type Holiday = 'newYear' | 'springFestival' | 'lantern' | 'dragonBoat' | 'qixi' | 'midAutumn'

export type SolarTerm =
  | 'xiaohan' | 'dahan' | 'lichun' | 'yushui' | 'jingzhe' | 'chunfen'
  | 'qingming' | 'guyu' | 'lixia' | 'xiaoman' | 'mangzhong' | 'xiazhi'
  | 'xiaoshu' | 'dashu' | 'liqiu' | 'chushu' | 'bailu' | 'qiufen'
  | 'hanlu' | 'shuangjiang' | 'lidong' | 'xiaoxue' | 'daxue' | 'dongzhi'

/** Festivals by Chinese calendar month and day (never in a leap month). */
const LUNAR_FESTIVALS: Record<string, Holiday> = {
  '1-1': 'springFestival',
  '1-15': 'lantern',
  '5-5': 'dragonBoat',
  '7-7': 'qixi',
  '8-15': 'midAutumn',
}

let lunarFormat: Intl.DateTimeFormat | null | undefined

/** The formatter, or null where the runtime has no Chinese calendar. */
function chineseCalendar(): Intl.DateTimeFormat | null {
  if (lunarFormat !== undefined) return lunarFormat
  try {
    const format = new Intl.DateTimeFormat('en-u-ca-chinese', { month: 'numeric', day: 'numeric' })
    lunarFormat = format.resolvedOptions().calendar === 'chinese' ? format : null
  } catch {
    lunarFormat = null
  }
  return lunarFormat
}

/** Month and day in the Chinese calendar; `leap` for a leap month ("5bis"). Null when unavailable. */
export function lunarDate(date: Date): { month: number; day: number; leap: boolean } | null {
  const format = chineseCalendar()
  if (!format) return null
  try {
    const parts = format.formatToParts(date)
    const month = parts.find((part) => part.type === 'month')?.value ?? ''
    const day = Number(parts.find((part) => part.type === 'day')?.value)
    const monthNumber = Number.parseInt(month, 10)
    if (!Number.isFinite(monthNumber) || !Number.isFinite(day)) return null
    return { month: monthNumber, day, leap: !/^\d+$/.test(month) }
  } catch {
    return null
  }
}

/**
 * The holiday on this local date. English shows only New Year's Day; Chinese adds
 * the lunar festivals, which quietly drop out where the calendar is unavailable.
 */
export function holidayOn(date: Date, language: string): Holiday | null {
  if (date.getMonth() === 0 && date.getDate() === 1) return 'newYear'
  if (!language.startsWith('zh')) return null
  const lunar = lunarDate(date)
  if (!lunar || lunar.leap) return null
  return LUNAR_FESTIVALS[`${lunar.month}-${lunar.day}`] ?? null
}

/**
 * The 21st-century constants of the usual solar-term formula, two per month
 * from January (小寒) to December (冬至):
 *   day = floor(Y × 0.2422 + C) − floor(L / 4)
 * where Y is the year's last two digits and L is Y, or Y − 1 for the January
 * and February terms (小寒, 大寒, 立春, 雨水). It gives the date in China
 * (UTC+8) and holds for 2000–2099 with the corrections below.
 */
const TERMS: [SolarTerm, number][] = [
  ['xiaohan', 5.4055], ['dahan', 20.12],
  ['lichun', 3.87], ['yushui', 18.73],
  ['jingzhe', 5.63], ['chunfen', 20.646],
  ['qingming', 4.81], ['guyu', 20.1],
  ['lixia', 5.52], ['xiaoman', 21.04],
  ['mangzhong', 5.678], ['xiazhi', 21.37],
  ['xiaoshu', 7.108], ['dashu', 22.83],
  ['liqiu', 7.5], ['chushu', 23.13],
  ['bailu', 7.646], ['qiufen', 23.042],
  ['hanlu', 8.318], ['shuangjiang', 23.438],
  ['lidong', 7.438], ['xiaoxue', 22.36],
  ['daxue', 7.18], ['dongzhi', 21.94],
]

/** Years where the formula is a day off, with the fix. */
const CORRECTIONS: Record<string, number> = {
  '2002:liqiu': 1,
  '2008:xiaoman': 1,
  '2016:xiaoshu': 1,
  '2019:xiaohan': -1,
  '2021:dongzhi': -1,
  '2026:yushui': -1,
  '2082:dahan': 1,
  '2084:chunfen': 1,
  '2089:shuangjiang': 1,
  '2089:lidong': 1,
}

/** Month (1–12) and day of a solar term, or null outside 2000–2099. */
export function solarTermDate(year: number, term: SolarTerm): { month: number; day: number } | null {
  if (year < 2000 || year > 2099) return null
  const index = TERMS.findIndex(([name]) => name === term)
  const [, c] = TERMS[index]
  const y = year % 100
  const l = index < 4 ? y - 1 : y
  const day = Math.floor(y * 0.2422 + c) - Math.floor(l / 4) + (CORRECTIONS[`${year}:${term}`] ?? 0)
  return { month: Math.floor(index / 2) + 1, day }
}

/** The solar term that falls on this local date, if any. */
export function solarTermOn(date: Date): SolarTerm | null {
  const year = date.getFullYear()
  const month = date.getMonth() + 1
  for (const index of [(month - 1) * 2, (month - 1) * 2 + 1]) {
    const term = TERMS[index][0]
    const at = solarTermDate(year, term)
    if (at && at.month === month && at.day === date.getDate()) return term
  }
  return null
}
