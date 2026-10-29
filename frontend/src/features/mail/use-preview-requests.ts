import { useEffect, useRef, useState } from 'react'
import { useDebouncedCallback } from '@tanstack/react-pacer'
import type { EmailSummary, PreviewLines } from '@/generated/accounts'
import { isGivenUp, nextGaveUpExpiry, useGaveUpVersion } from './preview-gave-up'

/** Quiet time after the rendered rows change before previews are requested. */
const REQUEST_DEBOUNCE_MS = 250

/** Rows whose preview the list is waiting for, given the rows it renders. */
export function wantedPreviewIds(rendered: EmailSummary[], previewLines: PreviewLines) {
  if (previewLines === 'none') return []
  return rendered
    .filter((email) => email.previewState === 'missing' && email.bodyState !== 'ready' && !isGivenUp(email.id))
    .map((email) => email.id)
    .sort()
}

/**
 * Asks the core for the previews of the rendered rows. The core keeps only the
 * latest request, so every call carries the complete wanted set; an empty set
 * is sent once after a non-empty one to cancel work for rows scrolled away.
 * Nothing is sent while previews are off, apart from that one cancelling call.
 */
export function usePreviewRequests(request: ((ids: string[]) => void) | undefined, rendered: EmailSummary[], previewLines: PreviewLines) {
  const gaveUpVersion = useGaveUpVersion()
  // Rows come back after their quiet period without any other change to prompt it.
  const [expiries, setExpiries] = useState(0)
  const sent = useRef<string | null>(null)
  const latestRequest = useRef(request)
  useEffect(() => { latestRequest.current = request }, [request])
  const wanted = wantedPreviewIds(rendered, previewLines)
  const key = wanted.join(',')

  const send = useDebouncedCallback(
    (ids: string[]) => {
      const joined = ids.join(',')
      if (joined === sent.current) return
      // Nothing outstanding and nothing to cancel.
      if (ids.length === 0 && !sent.current) return
      sent.current = joined
      latestRequest.current?.(ids)
    },
    { wait: REQUEST_DEBOUNCE_MS },
  )

  useEffect(() => {
    send(key === '' ? [] : key.split(','))
  }, [key, gaveUpVersion, expiries, send])

  // Wake up when the earliest given-up row becomes eligible again.
  useEffect(() => {
    const expiry = nextGaveUpExpiry()
    if (expiry === null) return
    const timer = setTimeout(() => setExpiries((count) => count + 1), Math.max(expiry - Date.now(), 0) + 1)
    return () => clearTimeout(timer)
  }, [gaveUpVersion, expiries])
}
