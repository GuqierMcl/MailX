import { HugeiconsIcon } from '@hugeicons/react'
import { cn } from '@/lib/utils'
import { SEGMENT_ICONS, type DaySegment } from './greeting'
import type { EntranceMode } from './motion'

const SEGMENTS = Object.keys(SEGMENT_ICONS) as DaySegment[]
const colorVar = (segment: DaySegment) => `var(--now-icon-${segment.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)})`

/**
 * The time-of-day icon at the start of the greeting, sized to the heading's capitals and sitting on
 * its baseline (it is an inline box, so the line's baseline is its own). Decorative: the greeting
 * words already name the time of day. All six icons are stacked and the current one is opaque, so a
 * change of segment while the page stays open cross-fades. On every visit the icon turns in from
 * -12 degrees as it fades in; with reduced motion it is simply there. Only opacity and transform move.
 */
export function GreetingIcon({ segment, mode }: { segment: DaySegment; mode: EntranceMode }) {
  return (
    <span
      aria-hidden
      data-slot="now-greeting-icon"
      data-segment={segment}
      className={cn('relative mr-2.5 inline-block size-[0.8em] align-[-0.06em]', mode !== 'none' && 'animate-now-icon-in')}
    >
      {SEGMENTS.map((name) => (
        <HugeiconsIcon
          key={name}
          icon={SEGMENT_ICONS[name]}
          strokeWidth={1.75}
          className={cn('absolute inset-0 size-full transition-opacity duration-500 ease-in-out motion-reduce:transition-none', name === segment ? 'opacity-100' : 'opacity-0')}
          style={{ color: colorVar(name) }}
        />
      ))}
    </span>
  )
}
