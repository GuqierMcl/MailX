import { useEffect, useState } from 'react'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { cn } from '@/lib/utils'
import { useDocumentVisible } from './use-document-visible'

/** How long the line takes to fade once the work is done (ai-chat-ui.md 4.6: about 300ms). */
export const WORKING_FADE_MS = 300

/**
 * A thin line that shows an AI task is running (ai-chat-ui.md 4.6): a bright segment glides across a
 * faint track while `active`, then the line fades out. It sits where its parent puts it (give the
 * parent `relative`) and takes no room. Only `transform` and `opacity` change; it stops when the
 * window is hidden and is a still line under reduced motion.
 */
export function AiWorkingLine({ active, className }: { active: boolean; className?: string }) {
  const reduced = usePrefersReducedMotion()
  const visible = useDocumentVisible()
  // Stays mounted through the fade so the glide does not freeze mid-way.
  const [faded, setFaded] = useState(!active)
  if (active && faded) setFaded(false)
  useEffect(() => {
    if (active) return
    const timer = setTimeout(() => setFaded(true), WORKING_FADE_MS)
    return () => clearTimeout(timer)
  }, [active])
  if (!active && faded) return null
  return (
    <div
      aria-hidden
      data-slot="ai-working-line"
      data-active={active}
      className={cn('pointer-events-none absolute inset-x-0 h-px overflow-hidden rounded-full bg-foreground/5 transition-opacity ease-out', active ? 'opacity-100' : 'opacity-0', className)}
      style={{ transitionDuration: `${reduced ? 100 : WORKING_FADE_MS}ms` }}
    >
      {reduced
        ? <div className="h-full w-full opacity-60" style={{ backgroundImage: 'var(--ai-flow)' }} />
        : <div className={cn('h-full w-2/5 animate-ai-flow', !visible && '[animation-play-state:paused]')} style={{ backgroundImage: 'var(--ai-flow)' }} />}
    </div>
  )
}
