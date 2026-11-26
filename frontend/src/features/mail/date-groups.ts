import type { WeekStartsOn } from '@/generated/accounts'

export type DateGroup = 'today' | 'yesterday' | 'thisWeek' | 'lastWeek' | 'earlier' | 'future'

const weekdays: Record<WeekStartsOn, number> = {
  sunday: 0,
  monday: 1,
  tuesday: 2,
  wednesday: 3,
  thursday: 4,
  friday: 5,
  saturday: 6,
}

export function dateGroupBoundaries(now: Date, weekStartsOn: WeekStartsOn) {
  const year = now.getFullYear()
  const month = now.getMonth()
  const date = now.getDate()
  const daysFromWeekStart = (now.getDay() - weekdays[weekStartsOn] + 7) % 7
  // Construct each local midnight separately so short and long days stay in their calendar dates.
  const startOfDay = (offset: number) => new Date(year, month, date + offset).getTime()

  return {
    year,
    tomorrow: startOfDay(1),
    today: startOfDay(0),
    yesterday: startOfDay(-1),
    thisWeek: startOfDay(-daysFromWeekStart),
    lastWeek: startOfDay(-daysFromWeekStart - 7),
  }
}

type DateGroupBoundaries = ReturnType<typeof dateGroupBoundaries>

export function emailDateGroup(receivedAt: string, bounds: DateGroupBoundaries): DateGroup {
  const received = Date.parse(receivedAt)
  if (received >= bounds.tomorrow) return 'future'
  if (received >= bounds.today) return 'today'
  if (received >= bounds.yesterday) return 'yesterday'
  if (received >= bounds.thisWeek) return 'thisWeek'
  if (received >= bounds.lastWeek) return 'lastWeek'
  return 'earlier'
}

export function formatGroupedListTime(receivedAt: string, group: DateGroup, bounds: DateGroupBoundaries, locale: string) {
  const date = new Date(receivedAt)
  if (Number.isNaN(date.getTime())) return ''
  if (group === 'today' || group === 'yesterday') {
    return date.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
  }
  return date.toLocaleDateString(locale, {
    month: 'numeric',
    day: 'numeric',
    year: date.getFullYear() === bounds.year ? undefined : 'numeric',
  })
}
