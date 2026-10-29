import { useSyncExternalStore } from 'react'

/** How long a row whose preview attempt ended without a result is left alone. */
export const GAVE_UP_MS = 60_000

const gaveUp = new Map<string, number>()
const listeners = new Set<() => void>()
// Bumped on every change, so subscribers re-read a fresh snapshot.
let version = 0

function changed() {
  version += 1
  for (const listener of listeners) listener()
}

/** Records that an attempt for `emailId` ended without a preview. */
export function giveUpPreview(emailId: string, now = Date.now()) {
  gaveUp.set(emailId, now)
  changed()
}

/** Whether `emailId` is still within its quiet period; expired entries are dropped. */
export function isGivenUp(emailId: string, now = Date.now()) {
  const at = gaveUp.get(emailId)
  if (at === undefined) return false
  if (now - at < GAVE_UP_MS) return true
  gaveUp.delete(emailId)
  return false
}

/** The earliest moment a recorded entry expires, or null when there is none. */
export function nextGaveUpExpiry() {
  let earliest: number | null = null
  for (const at of gaveUp.values()) if (earliest === null || at < earliest) earliest = at
  return earliest === null ? null : earliest + GAVE_UP_MS
}

/** Forgets everything; for tests. */
export function resetGaveUp() {
  gaveUp.clear()
  changed()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** Re-renders the caller whenever a preview is given up on or the quiet period is reset. */
export function useGaveUpVersion() {
  return useSyncExternalStore(subscribe, () => version)
}
