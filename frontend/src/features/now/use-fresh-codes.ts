import { useEffect, useState } from 'react'
import type { OverviewCode } from '@/generated/accounts'

/** Codes are shown for fifteen minutes after they arrive, like the core's window. */
export const CODE_LIFETIME_MS = 15 * 60_000

/**
 * The codes still fresh now. One timer waits for the next code to expire and
 * drops it, so a page left open does not keep showing an old code; no polling.
 */
export function useFreshCodes(codes: OverviewCode[]): OverviewCode[] {
  const [now, setNow] = useState(() => Date.now())
  const fresh = codes.filter((code) => Date.parse(code.receivedAt) + CODE_LIFETIME_MS > now)
  const nextExpiry = Math.min(...fresh.map((code) => Date.parse(code.receivedAt) + CODE_LIFETIME_MS))
  useEffect(() => {
    if (!Number.isFinite(nextExpiry)) return
    const timer = setTimeout(() => setNow(Date.now()), Math.max(0, nextExpiry - Date.now()) + 50)
    return () => clearTimeout(timer)
  }, [nextExpiry])
  return fresh
}
