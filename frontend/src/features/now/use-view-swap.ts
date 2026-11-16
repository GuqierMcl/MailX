import { useEffect, useState } from 'react'

/** Which way the views are moving; null when they are at rest. */
export type SwapPhase = 'to-conversation' | 'to-overview' | null

/** The longer of the two animations (in 250ms, out 170ms); the phase ends a beat after it. */
export const SWAP_MS = 250
const SETTLE_MS = 30

/**
 * Tells the Now page when the overview and the conversation trade places (ai-chat-ui.md 6.2). Both
 * views are live DOM and only their `transform` and `opacity` move (CSS classes keyed on the phase),
 * so text stays crisp and colours stay true; a View Transition would animate blurred snapshots.
 * The phase starts in the same render that sees the change, and ends on a
 * timer, so the classes (and `will-change`) are gone as soon as the motion is. Reduced motion never
 * enters a phase: the swap is instant.
 */
export function useViewSwap(conversation: boolean, reduced: boolean): SwapPhase {
  const [phase, setPhase] = useState<SwapPhase>(null)
  const [seen, setSeen] = useState(conversation)
  // Noticed while rendering, so the very first frame of the new view already carries the motion.
  if (seen !== conversation) {
    setSeen(conversation)
    setPhase(reduced ? null : conversation ? 'to-conversation' : 'to-overview')
  }
  useEffect(() => {
    if (phase === null) return
    const timer = setTimeout(() => setPhase(null), SWAP_MS + SETTLE_MS)
    return () => clearTimeout(timer)
  }, [phase])
  return reduced ? null : phase
}
