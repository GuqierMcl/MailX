import type { ReactNode } from 'react'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'

/**
 * Renders a loading placeholder only if it is still needed after a short
 * delay, then fades it in. Local reads usually finish first, so the common
 * case shows nothing instead of a flash.
 */
export function Delayed({ children, delay = 150 }: { children: ReactNode; delay?: number }) {
  const shown = useDelayedFlag(true, delay, 0)
  return shown ? <div className="animate-in duration-150 fade-in-0">{children}</div> : null
}
