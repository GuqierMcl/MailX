import type { CSSProperties } from 'react'
import { cn } from '@/lib/utils'
import type { DaySegment } from './greeting'

/** `token` at `percent`, scaled by the theme's `--now-glow`, over transparent. */
const tint = (token: string, percent: number) =>
  `color-mix(in oklch, var(${token}) calc(${percent}% * var(--now-glow)), transparent)`
const glow = (shape: string, token: string, percent: number) =>
  `radial-gradient(${shape}, ${tint(token, percent)}, transparent 70%)`

/**
 * Soft light from the top of the island, by part of the day: warm at dawn,
 * clear in the morning, even at noon, golden in the afternoon, dusky in the
 * evening and deep at night. The accent color (`--primary`) runs through all of
 * them; everything fades out before the text below, which keeps its contrast.
 */
const BACKGROUNDS: Record<DaySegment, string> = {
  earlyMorning: [
    glow('70% 55% at 12% 0%', '--color-amber-300', 30),
    glow('55% 45% at 88% 4%', '--color-rose-300', 20),
    glow('80% 50% at 50% 0%', '--primary', 8),
  ].join(', '),
  morning: [
    glow('65% 50% at 85% 0%', '--color-sky-300', 24),
    glow('70% 50% at 12% 0%', '--primary', 10),
  ].join(', '),
  noon: [
    glow('90% 45% at 50% 0%', '--color-amber-200', 16),
    glow('60% 40% at 50% 0%', '--primary', 8),
  ].join(', '),
  afternoon: [
    glow('65% 50% at 88% 2%', '--color-amber-300', 22),
    glow('60% 45% at 10% 0%', '--primary', 10),
  ].join(', '),
  evening: [
    glow('65% 50% at 90% 0%', '--color-orange-300', 22),
    glow('60% 50% at 8% 6%', '--color-violet-400', 18),
    glow('80% 45% at 50% 0%', '--primary', 8),
  ].join(', '),
  night: [
    glow('70% 55% at 15% 0%', '--color-indigo-500', 24),
    glow('55% 45% at 90% 8%', '--color-violet-500', 14),
    glow('80% 50% at 50% 0%', '--foreground', 5),
  ].join(', '),
}

const SEGMENTS = Object.keys(BACKGROUNDS) as DaySegment[]

/**
 * The page's backdrop. Static: when the part of the day changes, the layers
 * cross-fade once over a second (not with reduced motion), and nothing runs
 * in between. Hidden from screen readers and from the pointer.
 */
export function NowAmbient({ segment }: { segment: DaySegment }) {
  return (
    <div
      aria-hidden
      data-slot="now-ambient"
      data-segment={segment}
      className="pointer-events-none absolute inset-0 overflow-hidden [--now-glow:1] max-md:[--now-glow:0.6] dark:[--now-glow:0.75]"
    >
      {SEGMENTS.map((name) => (
        <div
          key={name}
          className={cn(
            'absolute inset-0 transition-opacity duration-1000 ease-in-out motion-reduce:transition-none',
            name === segment ? 'opacity-100' : 'opacity-0',
          )}
          style={{ backgroundImage: BACKGROUNDS[name] } satisfies CSSProperties}
        />
      ))}
    </div>
  )
}
