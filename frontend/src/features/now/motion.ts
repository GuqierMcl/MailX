import { useEffect, useState, type CSSProperties } from 'react'
import { cn } from '@/lib/utils'

const COUNT_MS = 600
const STAGGER_MS = 40

/**
 * A number that counts up from zero once, over ~600ms, when `enabled`; later
 * changes show at once. Only the text changes, one frame at a time.
 */
export function useCountUp(value: number, enabled: boolean): number {
  const [frame, setFrame] = useState<number | null>(null)
  const [done, setDone] = useState(!enabled)
  useEffect(() => {
    if (done || !enabled || value <= 0) return
    const start = performance.now()
    let id = requestAnimationFrame(function step(now) {
      const progress = Math.min(1, (now - start) / COUNT_MS)
      if (progress >= 1) {
        setFrame(null)
        setDone(true)
        return
      }
      setFrame(Math.round(value * (1 - (1 - progress) ** 3)))
      id = requestAnimationFrame(step)
    })
    return () => cancelAnimationFrame(id)
  }, [done, enabled, value])
  if (!enabled || done) return value
  return frame ?? 0
}

/**
 * How the page comes in: `first` is the day's first visit (a fuller entrance with counting numbers),
 * `return` every other visit (a lighter one), `none` for reduced motion.
 */
export type EntranceMode = 'first' | 'return' | 'none'

const RETURN_STAGGER_MS = 25

export function entranceMode(firstToday: boolean, reducedMotion: boolean): EntranceMode {
  return reducedMotion ? 'none' : firstToday ? 'first' : 'return'
}

/**
 * Entrance of one block. First visit: fade in with a 4px rise, 200ms, 40ms after the block before.
 * Return visit: a 2px rise, 150ms, 25ms apart. Both decelerate; only opacity and transform move.
 */
export function entrance(mode: EntranceMode, index: number): { className?: string; style?: CSSProperties } {
  if (mode === 'none') return {}
  if (mode === 'return') return { className: 'animate-now-rise', style: { animationDelay: `${index * RETURN_STAGGER_MS}ms` } }
  return {
    className: cn('animate-in fade-in-0 slide-in-from-bottom-1 fill-mode-both duration-200 ease-[cubic-bezier(0.16,1,0.3,1)]'),
    style: { animationDelay: `${index * STAGGER_MS}ms` },
  }
}
