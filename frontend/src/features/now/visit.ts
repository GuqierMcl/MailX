import { useEffect, useState } from 'react'
import { localDay, nextBoundary } from './greeting'

export const LAST_VISIT_KEY = 'mailx.now.lastVisitedAt'
export const ENTRANCE_KEY = 'mailx.now.entranceDay'

function read(key: string): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null
  } catch {
    return null
  }
}

function write(key: string, value: string) {
  try {
    globalThis.localStorage?.setItem(key, value)
  } catch {
    // Private storage or a full quota: the page still works, it just forgets.
  }
}

/** When the user last looked at the page; on first use, the start of today (local time). */
export function readLastVisit(now = new Date()): string {
  const saved = read(LAST_VISIT_KEY)
  if (saved && !Number.isNaN(Date.parse(saved))) return new Date(saved).toISOString()
  const start = new Date(now)
  start.setHours(0, 0, 0, 0)
  return start.toISOString()
}

export function writeLastVisit(now = new Date()) {
  write(LAST_VISIT_KEY, now.toISOString())
}

/**
 * The `since` of this visit, fixed for as long as the page stays open. Leaving
 * the page (it unmounts) or hiding the window while on it records the visit, so
 * the next one counts from here. Several windows share the key: the latest wins.
 */
export function useLastVisit(): string {
  const [since] = useState(() => readLastVisit())
  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') writeLastVisit()
    }
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      document.removeEventListener('visibilitychange', onVisibility)
      writeLastVisit()
    }
  }, [])
  return since
}

/** Whether this is the first visit of the local day; the answer holds for this visit. */
export function useFirstVisitToday(): boolean {
  const [first] = useState(() => read(ENTRANCE_KEY) !== localDay(new Date()))
  useEffect(() => {
    write(ENTRANCE_KEY, localDay(new Date()))
  }, [])
  return first
}

/**
 * The current time, updated only when the greeting can change (a segment
 * boundary or midnight): one timer at a time, no polling.
 */
export function useDayClock(): Date {
  const [now, setNow] = useState(() => new Date())
  useEffect(() => {
    const delay = Math.max(1000, nextBoundary(now).getTime() - Date.now() + 1000)
    const timer = setTimeout(() => setNow(new Date()), delay)
    return () => clearTimeout(timer)
  }, [now])
  return now
}
