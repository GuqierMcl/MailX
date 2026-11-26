import { useCallback, useEffect, useState } from 'react'
import { localDay } from '../greeting'

/** Modules hidden for today: id to the day and the content signature they were hidden at. */
export const SNOOZED_KEY = 'mailx.now.snoozed'

export type Snoozed = Record<string, { day: string; signature: string }>

const DAY = /^\d{4}-\d{2}-\d{2}$/

function read(): string | null {
  try {
    return globalThis.localStorage?.getItem(SNOOZED_KEY) ?? null
  } catch {
    return null
  }
}

function write(value: Snoozed) {
  try {
    if (Object.keys(value).length === 0) globalThis.localStorage?.removeItem(SNOOZED_KEY)
    else globalThis.localStorage?.setItem(SNOOZED_KEY, JSON.stringify(value))
  } catch {
    // Private storage or a full quota: the module just comes back next time.
  }
}

/** The entries of `text` that are well formed and from today; anything else (old days, bad JSON) is dropped. */
export function parseSnoozed(text: string | null, now = new Date()): Snoozed {
  if (!text) return {}
  const today = localDay(now)
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return {}
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {}
  const kept: Snoozed = {}
  for (const [id, entry] of Object.entries(parsed)) {
    if (typeof entry !== 'object' || entry === null) continue
    const { day, signature } = entry as Record<string, unknown>
    if (typeof day === 'string' && DAY.test(day) && day === today && typeof signature === 'string') kept[id] = { day, signature }
  }
  return kept
}

export function readSnoozed(now = new Date()): Snoozed {
  return parseSnoozed(read(), now)
}

/** Hides `id` for the rest of the day at this signature; expired entries are dropped on the way. */
export function snoozeModule(id: string, signature: string, now = new Date()): Snoozed {
  const next = { ...readSnoozed(now), [id]: { day: localDay(now), signature } }
  write(next)
  return next
}

/** Brings `id` back right away. */
export function unsnoozeModule(id: string, now = new Date()): Snoozed {
  const next = readSnoozed(now)
  delete next[id]
  write(next)
  return next
}

/**
 * The page's view of the store, read once on entry and kept in step with
 * what this page writes and with other windows. Expiry at midnight is the
 * caller's: entries keep their day, and `suppressed` compares it with today.
 */
export function useSnoozed() {
  const [snoozed, setSnoozed] = useState<Snoozed>(() => readSnoozed())
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === null || event.key === SNOOZED_KEY) setSnoozed(readSnoozed())
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [])
  const snooze = useCallback((id: string, signature: string) => setSnoozed(snoozeModule(id, signature)), [])
  const restore = useCallback((id: string) => setSnoozed(unsnoozeModule(id)), [])
  return { snoozed, snooze, restore }
}
