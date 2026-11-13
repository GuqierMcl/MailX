import type { ParseKeys } from 'i18next'
import { holidayOn, solarTermOn, type Holiday, type SolarTerm } from './calendar'

/** A message key of the `now` namespace. */
export type NowKey = Extract<ParseKeys<['now']>, `now:${string}`>

/** Parts of the local day; each has its own greeting and background. */
export type DaySegment = 'earlyMorning' | 'morning' | 'noon' | 'afternoon' | 'evening' | 'night'

/** Start hour of each segment; night wraps past midnight to 5:00. */
const SEGMENT_STARTS: [number, DaySegment][] = [
  [5, 'earlyMorning'], [8, 'morning'], [11, 'noon'], [13, 'afternoon'], [17, 'evening'], [21, 'night'],
]

export function daySegment(date: Date): DaySegment {
  const hour = date.getHours()
  let segment: DaySegment = 'night'
  for (const [start, name] of SEGMENT_STARTS) if (hour >= start) segment = name
  return segment
}

/** The next moment the greeting may change: a segment boundary or midnight (the date and line). */
export function nextBoundary(date: Date): Date {
  const hours = [...SEGMENT_STARTS.map(([start]) => start), 24]
  const next = hours.find((hour) => hour > date.getHours()) ?? 24
  const at = new Date(date)
  at.setHours(next, 0, 0, 0)
  return at
}

/** Local calendar date as `YYYY-MM-DD`. */
export function localDay(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
}

export type LineContext =
  | { kind: 'attention' }
  | { kind: 'holiday'; holiday: Holiday }
  | { kind: 'solarTerm'; term: SolarTerm }
  | { kind: 'newMail'; count: number }
  | { kind: 'monday' }
  | { kind: 'friday' }
  | { kind: 'weekend' }
  | { kind: 'clear' }
  | { kind: 'steady' }

/** Lines per group in `now.json` (`a`, `b`, `c`, ...); both languages have the same set. */
const LINE_COUNTS: Record<LineContext['kind'], number> = {
  attention: 3, holiday: 3, solarTerm: 3, newMail: 3, monday: 3, friday: 3, weekend: 3, clear: 3, steady: 3,
}

export type MailState = {
  /** Account, send or similar problems that need the user (not drafts, which never clear on their own). */
  attention: boolean
  newMail: number
  unread: number
}

/**
 * Which group the summary line comes from. Priority, highest first:
 * 1. attention: something is broken or stuck; the line points at it.
 * 2. holiday, then solar term (Chinese only): the day itself is the news.
 * 3. new mail since the last visit, with its count.
 * 4. Monday, Friday, the weekend.
 * 5. clear (no unread inbox mail), else steady (nothing new, some unread left).
 * A holiday beats new mail because the count is right below in its own section.
 */
export function lineContext(date: Date, language: string, state: MailState): LineContext {
  if (state.attention) return { kind: 'attention' }
  const holiday = holidayOn(date, language)
  if (holiday) return { kind: 'holiday', holiday }
  if (language.startsWith('zh')) {
    const term = solarTermOn(date)
    if (term) return { kind: 'solarTerm', term }
  }
  if (state.newMail > 0) return { kind: 'newMail', count: state.newMail }
  const weekday = date.getDay()
  if (weekday === 1) return { kind: 'monday' }
  if (weekday === 5) return { kind: 'friday' }
  if (weekday === 0 || weekday === 6) return { kind: 'weekend' }
  return state.unread > 0 ? { kind: 'steady' } : { kind: 'clear' }
}

/** FNV-1a: small, stable across runs and platforms. */
function hash(text: string): number {
  let value = 0x811c9dc5
  for (let i = 0; i < text.length; i++) {
    value ^= text.charCodeAt(i)
    value = Math.imul(value, 0x01000193)
  }
  return value >>> 0
}

/**
 * The message key of the line to show, picked from the group by the local date
 * and the context: the same all day, until the context changes. The count of
 * new mail is not part of the seed, so a new message keeps the sentence.
 */
export function lineKey(date: Date, context: LineContext): NowKey {
  const count = LINE_COUNTS[context.kind]
  const letter = String.fromCharCode(97 + (hash(`${localDay(date)}|${context.kind}`) % count))
  return `now:lines.${context.kind}.${letter}` as NowKey
}
