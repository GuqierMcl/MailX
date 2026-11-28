import { useEffect, useState } from 'react'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { cn } from '@/lib/utils'
import { resolveAmbientScene, type NowAmbientMode, type NowAmbientScene, type NowConcreteScene } from './ambient-preference'
import { AmbientSceneArt } from './ambient-scenes'
import type { DayOccasion } from './calendar'
import type { DaySegment } from './greeting'

type OccasionEffect = 'snow' | 'rain' | 'fullMoon' | 'festive'

function occasionEffect(occasion: DayOccasion | null): OccasionEffect | null {
  if (!occasion) return null
  if (occasion.kind === 'holiday') {
    if (occasion.holiday === 'midAutumn' || occasion.holiday === 'lantern') return 'fullMoon'
    if (occasion.holiday === 'springFestival' || occasion.holiday === 'newYear') return 'festive'
    return null
  }
  switch (occasion.term) {
    case 'xiaoxue':
    case 'daxue':
    case 'xiaohan':
    case 'dahan':
    case 'dongzhi':
      return 'snow'
    case 'yushui':
    case 'qingming':
    case 'guyu':
      return 'rain'
    default:
      return null
  }
}

const SNOWFLAKES: { x: number; y: number; r: number; delay: string }[] = [
  { x: 10, y: 8, r: 2.2, delay: '0s' },
  { x: 22, y: 18, r: 1.6, delay: '-5s' },
  { x: 36, y: 6, r: 2.0, delay: '-11s' },
  { x: 52, y: 14, r: 1.5, delay: '-3s' },
  { x: 68, y: 10, r: 2.3, delay: '-8s' },
  { x: 80, y: 20, r: 1.7, delay: '-15s' },
  { x: 91, y: 12, r: 2.0, delay: '-19s' },
]

const RAINDROPS: { x: number; y: number; len: number; delay: string }[] = [
  { x: 14, y: 6, len: 26, delay: '0s' },
  { x: 28, y: 14, len: 20, delay: '-4s' },
  { x: 44, y: 8, len: 24, delay: '-9s' },
  { x: 66, y: 12, len: 22, delay: '-2s' },
  { x: 82, y: 6, len: 28, delay: '-7s' },
  { x: 93, y: 16, len: 18, delay: '-12s' },
]

const SWAP_MS = 1200

function useDocumentHidden() {
  const [hidden, setHidden] = useState(() => (typeof document !== 'undefined' ? document.hidden : false))
  useEffect(() => {
    if (typeof document === 'undefined') return
    const onChange = () => setHidden(document.hidden)
    document.addEventListener('visibilitychange', onChange)
    return () => document.removeEventListener('visibilitychange', onChange)
  }, [])
  return hidden
}

type SceneSnapshot = { scene: NowConcreteScene; segment: DaySegment }

/**
 * The Now page's preset SVG backdrop: fades in smoothly when entering the page,
 * cross-fades cleanly between scenes or day segments (only mounting the outgoing
 * scene during the 1.2s swap), and pauses all ambient motion when hidden or static.
 */
export function NowAmbient({ segment, scene = 'auto', occasion = null, mode = 'full', dimmed = false }: {
  segment: DaySegment
  scene?: NowAmbientScene
  occasion?: DayOccasion | null
  mode?: NowAmbientMode
  /** Softly recedes while the AI conversation covers the page, keeping long text crisp. */
  dimmed?: boolean
}) {
  const reduced = usePrefersReducedMotion()
  const hidden = useDocumentHidden()
  const resolved = resolveAmbientScene(scene, segment)
  const [current, setCurrent] = useState<SceneSnapshot>({ scene: resolved, segment })
  const [previous, setPrevious] = useState<SceneSnapshot | null>(null)

  if (resolved !== current.scene || segment !== current.segment) {
    setPrevious(current)
    setCurrent({ scene: resolved, segment })
  }
  useEffect(() => {
    if (previous === null) return
    const timer = setTimeout(() => setPrevious(null), SWAP_MS)
    return () => clearTimeout(timer)
  }, [previous])

  const animate = mode === 'full' && !reduced
  const paused = !animate || hidden || dimmed
  const effect = occasionEffect(occasion)

  if (mode === 'off') {
    return (
      <div
        aria-hidden
        data-slot="now-ambient"
        data-segment={segment}
        data-scene={resolved}
        data-mode="off"
        className="pointer-events-none absolute inset-0 overflow-hidden [contain:strict]"
      />
    )
  }

  return (
    <div
      aria-hidden
      data-slot="now-ambient"
      data-segment={segment}
      data-scene={resolved}
      data-mode={mode}
      data-occasion={effect ?? undefined}
      data-paused={paused ? 'true' : undefined}
      data-dimmed={dimmed ? 'true' : undefined}
      className={cn(
        'pointer-events-none absolute inset-0 overflow-hidden transition-opacity duration-300 ease-out [contain:strict] motion-reduce:transition-none',
        !reduced && 'now-ambient-enter',
        dimmed && 'opacity-25',
      )}
    >
      {/* Outgoing scene during a smooth cross-fade; unmounted once the swap ends. */}
      {previous && !reduced && (
        <div
          key={`prev-${previous.scene}-${previous.segment}`}
          className="animate-now-text-out absolute inset-0 [animation-duration:1200ms]"
        >
          <AmbientSceneArt scene={previous.scene} segment={previous.segment} animate={false} />
        </div>
      )}

      {/* Active preset SVG scene. */}
      <div
        key={`cur-${current.scene}-${current.segment}`}
        className={cn('absolute inset-0', previous && !reduced && 'animate-now-text-in [animation-duration:1200ms]')}
      >
        <AmbientSceneArt scene={current.scene} segment={current.segment} animate={animate} />
      </div>

      {/* Solar term / holiday light overlays: snow, rain, full moon or warm festive glow. */}
      {effect === 'fullMoon' && (
        <div
          data-slot="now-ambient-occasion"
          className={cn(
            'absolute top-10 right-[15%] size-24 rounded-full bg-[radial-gradient(circle,color-mix(in_oklch,var(--color-amber-200)_42%,transparent)_0%,color-mix(in_oklch,var(--color-amber-300)_16%,transparent)_58%,transparent_76%)]',
            animate && 'now-ambient-breathe',
          )}
        />
      )}
      {effect === 'snow' && (
        <svg data-slot="now-ambient-occasion" viewBox="0 0 100 50" preserveAspectRatio="xMidYMin slice" className="absolute inset-x-0 top-0 h-80 w-full">
          {SNOWFLAKES.map((flake, index) => (
            <circle
              key={index}
              cx={flake.x}
              cy={flake.y}
              r={flake.r * 0.28}
              style={{ animationDelay: flake.delay }}
              className={cn('fill-sky-400/35 dark:fill-sky-100/40', animate && 'now-ambient-fall')}
            />
          ))}
        </svg>
      )}
      {effect === 'rain' && (
        <svg data-slot="now-ambient-occasion" viewBox="0 0 100 50" preserveAspectRatio="xMidYMin slice" className="absolute inset-x-0 top-0 h-80 w-full">
          {RAINDROPS.map((drop, index) => (
            <line
              key={index}
              x1={drop.x}
              y1={drop.y}
              x2={drop.x - 1.2}
              y2={drop.y + drop.len * 0.18}
              strokeWidth={0.22}
              strokeLinecap="round"
              style={{ animationDelay: drop.delay }}
              className={cn('stroke-sky-500/25 dark:stroke-sky-300/30', animate && 'now-ambient-fall')}
            />
          ))}
        </svg>
      )}
      {effect === 'festive' && (
        <div
          data-slot="now-ambient-occasion"
          className={cn(
            'absolute inset-0 bg-[radial-gradient(50%_40%_at_80%_8%,color-mix(in_oklch,var(--color-rose-400)_16%,transparent),transparent_70%),radial-gradient(45%_35%_at_20%_12%,color-mix(in_oklch,var(--color-amber-300)_16%,transparent),transparent_70%)]',
            animate && 'now-ambient-breathe',
          )}
        />
      )}
    </div>
  )
}
