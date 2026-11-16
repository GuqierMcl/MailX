import { useCallback, useEffect, useRef, useState } from 'react'
import type { ContactSuggestion } from '@/generated/accounts'

/** Wait after the last keystroke before asking the core (sending.md section 17.4). */
export const SUGGEST_DELAY_MS = 80
/** Shown at most. */
export const SUGGEST_LIMIT = 8

export type SuggestContacts = (query: string, limit?: number) => Promise<ContactSuggestion[]>

/**
 * The suggestion state for one recipient input. `request(text)` schedules a
 * query; only the newest request's answer is ever shown, so a slow early answer
 * cannot replace a later one. Addresses in `exclude` (lowercase) never appear.
 */
export function useContactSuggestions(suggest: SuggestContacts | undefined, exclude: ReadonlySet<string>) {
  const [items, setItems] = useState<ContactSuggestion[]>([])
  const [active, setActive] = useState(0)
  const latest = useRef(0)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const excluded = useRef(exclude)
  useEffect(() => { excluded.current = exclude }, [exclude])

  const close = useCallback(() => {
    latest.current += 1
    clearTimeout(timer.current)
    setItems([])
    setActive(0)
  }, [])
  useEffect(() => () => {
    latest.current += 1
    clearTimeout(timer.current)
  }, [])

  const request = useCallback((text: string) => {
    const query = text.trim()
    const id = ++latest.current
    clearTimeout(timer.current)
    if (!suggest || !query) {
      setItems([])
      setActive(0)
      return
    }
    timer.current = setTimeout(() => {
      suggest(query, SUGGEST_LIMIT + excluded.current.size).then((found) => {
        if (id !== latest.current) return
        const next = found.filter((item) => !excluded.current.has(item.address.toLowerCase())).slice(0, SUGGEST_LIMIT)
        setItems(next)
        setActive(0)
      }, () => {
        if (id === latest.current) setItems([])
      })
    }, SUGGEST_DELAY_MS)
  }, [suggest])

  // An address added to any row meanwhile leaves the open list too.
  const visible = items.filter((item) => !exclude.has(item.address.toLowerCase()))
  return {
    items: visible,
    active: Math.min(active, Math.max(visible.length - 1, 0)),
    open: visible.length > 0,
    request,
    close,
    move: (step: number) => setActive((current) => visible.length ? (Math.min(current, visible.length - 1) + step + visible.length) % visible.length : 0),
    setActive,
  }
}
