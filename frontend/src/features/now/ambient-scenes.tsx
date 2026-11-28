import { useId } from 'react'
import { cn } from '@/lib/utils'
import type { NowConcreteScene } from './ambient-preference'
import type { DaySegment } from './greeting'

type LinePalette = {
  /** Theme-adaptive CSS custom property classes applied on `<svg>` so SVG gradients adapt to light/dark modes. */
  varsClass: string
  inkStroke: string
  inkFill: string
}

/**
 * Fine-line semantic palettes across 6 day segments:
 * - Light mode uses refined 500–950 mineral & ink tones for razor-sharp etching on white paper.
 * - Dark mode shifts automatically to luminous 100–300 tints for calm glowing wireframes.
 */
const PALETTES: Record<DaySegment, LinePalette> = {
  earlyMorning: {
    varsClass:
      '[--line-a:var(--color-amber-600)] [--line-b:var(--color-rose-500)] [--line-hi:var(--color-orange-500)] [--line-ink:var(--color-amber-950)] [--line-glow:var(--color-amber-300)] dark:[--line-a:var(--color-amber-200)] dark:[--line-b:var(--color-rose-300)] dark:[--line-hi:var(--color-amber-100)] dark:[--line-ink:var(--color-amber-100)] dark:[--line-glow:var(--color-amber-400)]',
    inkStroke: 'stroke-amber-950 dark:stroke-amber-100',
    inkFill: 'fill-amber-950 dark:fill-amber-100',
  },
  morning: {
    varsClass:
      '[--line-a:var(--color-sky-600)] [--line-b:var(--color-cyan-600)] [--line-hi:var(--color-amber-500)] [--line-ink:var(--color-sky-950)] [--line-glow:var(--color-sky-300)] dark:[--line-a:var(--color-sky-200)] dark:[--line-b:var(--color-cyan-300)] dark:[--line-hi:var(--color-amber-100)] dark:[--line-ink:var(--color-sky-100)] dark:[--line-glow:var(--color-sky-400)]',
    inkStroke: 'stroke-sky-950 dark:stroke-sky-100',
    inkFill: 'fill-sky-950 dark:fill-sky-100',
  },
  noon: {
    varsClass:
      '[--line-a:var(--color-sky-600)] [--line-b:var(--color-teal-600)] [--line-hi:var(--color-amber-500)] [--line-ink:var(--color-sky-950)] [--line-glow:var(--color-sky-200)] dark:[--line-a:var(--color-sky-200)] dark:[--line-b:var(--color-teal-300)] dark:[--line-hi:var(--color-amber-100)] dark:[--line-ink:var(--color-sky-100)] dark:[--line-glow:var(--color-teal-400)]',
    inkStroke: 'stroke-sky-950 dark:stroke-sky-100',
    inkFill: 'fill-sky-950 dark:fill-sky-100',
  },
  afternoon: {
    varsClass:
      '[--line-a:var(--color-amber-600)] [--line-b:var(--color-orange-500)] [--line-hi:var(--color-amber-500)] [--line-ink:var(--color-amber-950)] [--line-glow:var(--color-amber-300)] dark:[--line-a:var(--color-amber-200)] dark:[--line-b:var(--color-orange-300)] dark:[--line-hi:var(--color-amber-100)] dark:[--line-ink:var(--color-amber-100)] dark:[--line-glow:var(--color-orange-400)]',
    inkStroke: 'stroke-amber-950 dark:stroke-amber-100',
    inkFill: 'fill-amber-950 dark:fill-amber-100',
  },
  evening: {
    varsClass:
      '[--line-a:var(--color-rose-500)] [--line-b:var(--color-violet-600)] [--line-hi:var(--color-orange-500)] [--line-ink:var(--color-violet-950)] [--line-glow:var(--color-rose-400)] dark:[--line-a:var(--color-orange-200)] dark:[--line-b:var(--color-rose-300)] dark:[--line-hi:var(--color-amber-100)] dark:[--line-ink:var(--color-rose-100)] dark:[--line-glow:var(--color-violet-500)]',
    inkStroke: 'stroke-violet-950 dark:stroke-rose-100',
    inkFill: 'fill-violet-950 dark:fill-rose-100',
  },
  night: {
    varsClass:
      '[--line-a:var(--color-indigo-600)] [--line-b:var(--color-violet-600)] [--line-hi:var(--color-indigo-700)] [--line-ink:var(--color-indigo-950)] [--line-glow:var(--color-indigo-400)] dark:[--line-a:var(--color-indigo-200)] dark:[--line-b:var(--color-violet-300)] dark:[--line-hi:var(--color-indigo-100)] dark:[--line-ink:var(--color-indigo-100)] dark:[--line-glow:var(--color-indigo-500)]',
    inkStroke: 'stroke-indigo-950 dark:stroke-indigo-100',
    inkFill: 'fill-indigo-950 dark:fill-indigo-100',
  },
}

export type SceneProps = {
  scene: NowConcreteScene
  segment: DaySegment
  animate: boolean
  /** Compact thumbnail mode used inside Settings → Now. */
  preview?: boolean
}

/**
 * Four pre-composed fine-line vector scenes. Built from stroke gradients and
 * precision geometry (zero SVG filters or CSS backdrop-filter) for 60fps resize.
 */
export function AmbientSceneArt({ scene, segment, animate, preview = false }: SceneProps) {
  const uid = useId().replace(/:/g, '')
  const palette = PALETTES[segment]
  const night = segment === 'night'
  const boost = preview ? 1.55 : 1

  switch (scene) {
    case 'peaks':
      return <PeaksScene uid={uid} palette={palette} night={night} animate={animate} boost={boost} preview={preview} />
    case 'sea':
      return <SeaScene uid={uid} palette={palette} night={night} animate={animate} boost={boost} preview={preview} />
    case 'window':
      return <WindowScene uid={uid} palette={palette} night={night} animate={animate} boost={boost} preview={preview} />
    case 'orbit':
      return <OrbitScene uid={uid} palette={palette} night={night} animate={animate} boost={boost} preview={preview} />
  }
}

type InnerProps = {
  uid: string
  palette: LinePalette
  night: boolean
  animate: boolean
  boost: number
  preview: boolean
}

function alpha(base: number, boost: number, max = 0.92): number {
  return Math.min(max, Number((base * boost).toFixed(3)))
}

function ellipsePoint(cx: number, cy: number, rx: number, ry: number, angleDeg: number, thetaRad: number) {
  const rad = (angleDeg * Math.PI) / 180
  const cosA = Math.cos(rad)
  const sinA = Math.sin(rad)
  const ex = rx * Math.cos(thetaRad)
  const ey = ry * Math.sin(thetaRad)
  return {
    x: Number((cx + ex * cosA - ey * sinA).toFixed(1)),
    y: Number((cy + ex * sinA + ey * cosA).toFixed(1)),
  }
}

function fourPointStar(cx: number, cy: number, size: number): string {
  const inner = Number((size * 0.16).toFixed(2))
  const s = Number(size.toFixed(2))
  return [
    `M${cx},${cy - s}`,
    `Q${cx + inner},${cy - inner} ${cx + s},${cy}`,
    `Q${cx + inner},${cy + inner} ${cx},${cy + s}`,
    `Q${cx - inner},${cy + inner} ${cx - s},${cy}`,
    `Q${cx - inner},${cy - inner} ${cx},${cy - s}Z`,
  ].join(' ')
}

/** Evaluates a smooth sculpted ridge contour across x in [-160, 1360] with asymmetric massif spurs. */
function buildContourPath(index: number, total: number): string {
  const t = index / (total - 1)
  // Power curve spacing: sparser at upper distant peaks, progressively denser near foreground.
  const baseY = 392 + Math.pow(t, 0.84) * 296
  const amp = 1 - t * 0.36
  // Diagonal spur drift creates true topographic elevation depth across contours.
  const rightPeakX = 905 - t * 48 + Math.sin(t * Math.PI) * 32
  const leftPeakX = 215 + t * 88
  const xs = [-160, 20, leftPeakX, 430 + t * 20, 625, rightPeakX, 1135, 1360]
  const ys = xs.map((x) => {
    const rightMassif = -84 * amp * Math.exp(-Math.pow((x - rightPeakX) / (135 + t * 45), 2))
    const leftShoulder = -44 * (1 - t * 0.15) * Math.exp(-Math.pow((x - leftPeakX) / (155 + t * 35), 2))
    const midSaddle = 14 * Math.sin((x - 160) * 0.0052 + t * 1.4)
    const microRipple = 5 * Math.cos(x * 0.014 - t * 2.1)
    return Number((baseY + rightMassif + leftShoulder + midSaddle + microRipple).toFixed(1))
  })
  // Convert sampled points to smooth Catmull-Rom cubic Bezier segments.
  let d = `M${xs[0]},${ys[0]}`
  for (let i = 0; i < xs.length - 1; i++) {
    const x0 = xs[Math.max(0, i - 1)]
    const y0 = ys[Math.max(0, i - 1)]
    const x1 = xs[i]
    const y1 = ys[i]
    const x2 = xs[i + 1]
    const y2 = ys[i + 1]
    const x3 = xs[Math.min(xs.length - 1, i + 2)]
    const y3 = ys[Math.min(ys.length - 1, i + 2)]
    const c1x = Number((x1 + (x2 - x0) / 6).toFixed(1))
    const c1y = Number((y1 + (y2 - y0) / 6).toFixed(1))
    const c2x = Number((x2 - (x3 - x1) / 6).toFixed(1))
    const c2y = Number((y2 - (y3 - y1) / 6).toFixed(1))
    d += ` C${c1x},${c1y} ${c2x},${c2y} ${x2},${y2}`
  }
  return d
}

const PEAK_CONTOURS = Array.from({ length: 14 }, (_, index) => ({
  d: buildContourPath(index, 14),
  tier: index < 5 ? 'far' : index < 10 ? 'mid' : 'near',
  weight: index < 5 ? 0.8 : index < 10 ? 0.94 : 1.08,
  alphaScale: 0.68 + (index / 13) * 0.32,
}))

const PEAK_RAYS = [0, 45, 90, 135, 180, 225, 270, 315].map((deg) => {
  const rad = (deg * Math.PI) / 180
  const r1 = 36
  const r2 = deg % 90 === 0 ? 49 : 44
  return {
    x1: Number((305 + Math.cos(rad) * r1).toFixed(1)),
    y1: Number((150 + Math.sin(rad) * r1).toFixed(1)),
    x2: Number((305 + Math.cos(rad) * r2).toFixed(1)),
    y2: Number((150 + Math.sin(rad) * r2).toFixed(1)),
  }
})

/**
 * 1. Peaks & Contours: 14 topographic elevation streamlines (sparser in distance,
 * tighter near foreground), hollow concentric sun/moon rings with radial ticks, and cloud filaments.
 */
function PeaksScene({ uid, palette, night, animate, boost, preview }: InnerProps) {
  const sw = preview ? 1.6 : 1
  return (
    <svg
      data-slot="now-ambient-svg"
      data-scene="peaks"
      viewBox={preview ? '-50 44 1300 620' : '0 0 1200 720'}
      preserveAspectRatio={preview ? 'xMidYMid slice' : 'xMidYMax slice'}
      className={cn('absolute inset-0 size-full', palette.varsClass)}
    >
      <defs>
        <radialGradient id={`${uid}-air`} cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="var(--line-glow)" stopOpacity={alpha(0.11, boost)} />
          <stop offset="55%" stopColor="var(--line-glow)" stopOpacity={alpha(0.035, boost)} />
          <stop offset="100%" stopColor="var(--line-glow)" stopOpacity="0" />
        </radialGradient>
        <linearGradient id={`${uid}-contour-far`} x1="0" y1="0.5" x2="1" y2="0.5">
          <stop offset="0%" stopColor="var(--line-b)" stopOpacity="0" />
          <stop offset="18%" stopColor="var(--line-b)" stopOpacity={alpha(0.3, boost)} />
          <stop offset="50%" stopColor="var(--line-b)" stopOpacity={alpha(0.2, boost)} />
          <stop offset="76%" stopColor="var(--line-hi)" stopOpacity={alpha(0.36, boost)} />
          <stop offset="100%" stopColor="var(--line-b)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-contour-mid`} x1="0" y1="0.5" x2="1" y2="0.5">
          <stop offset="0%" stopColor="var(--line-a)" stopOpacity="0" />
          <stop offset="18%" stopColor="var(--line-a)" stopOpacity={alpha(0.3, boost)} />
          <stop offset="50%" stopColor="var(--line-a)" stopOpacity={alpha(0.18, boost)} />
          <stop offset="76%" stopColor="var(--line-hi)" stopOpacity={alpha(0.36, boost)} />
          <stop offset="100%" stopColor="var(--line-a)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-contour-near`} x1="0" y1="0.5" x2="1" y2="0.5">
          <stop offset="0%" stopColor="var(--line-ink)" stopOpacity="0" />
          <stop offset="20%" stopColor="var(--line-ink)" stopOpacity={alpha(0.34, boost)} />
          <stop offset="52%" stopColor="var(--line-a)" stopOpacity={alpha(0.22, boost)} />
          <stop offset="80%" stopColor="var(--line-ink)" stopOpacity={alpha(0.36, boost)} />
          <stop offset="100%" stopColor="var(--line-ink)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-filament`} x1="0" y1="0.5" x2="1" y2="0.5">
          <stop offset="0%" stopColor="var(--line-b)" stopOpacity="0" />
          <stop offset="30%" stopColor="var(--line-a)" stopOpacity={alpha(0.26, boost)} />
          <stop offset="70%" stopColor="var(--line-hi)" stopOpacity={alpha(0.24, boost)} />
          <stop offset="100%" stopColor="var(--line-b)" stopOpacity="0" />
        </linearGradient>
      </defs>

      {/* Single whisper-faint ambient air glow */}
      <circle cx="305" cy="150" r="270" fill={`url(#${uid}-air)`} />

      {/* Hollow concentric sun/moon reticle & radial tick rays */}
      <g
        fill="none"
        strokeLinecap="round"
        className={cn(animate && 'now-ambient-breathe')}
        style={{ transformOrigin: '305px 150px' }}
      >
        <circle cx="305" cy="150" r="98" stroke="var(--line-b)" strokeOpacity={alpha(0.12, boost)} strokeWidth={0.75 * sw} />
        <circle cx="305" cy="150" r="64" stroke="var(--line-a)" strokeOpacity={alpha(0.2, boost)} strokeWidth={0.85 * sw} strokeDasharray="2 6" />
        <circle cx="305" cy="150" r="42" stroke="var(--line-hi)" strokeOpacity={alpha(0.24, boost)} strokeWidth={0.85 * sw} />
        {night ? (
          <>
            <path
              d="M311,126 A25,25 0 1,0 326,164 A21,21 0 1,1 311,126 Z"
              stroke="var(--line-hi)"
              strokeOpacity={alpha(0.52, boost)}
              strokeWidth={1.15 * sw}
              strokeLinejoin="round"
            />
            <path
              d="M311,126 A25,25 0 0,1 326,164"
              stroke="var(--line-a)"
              strokeOpacity={alpha(0.18, boost)}
              strokeWidth={0.75 * sw}
              strokeDasharray="1.5 3.5"
            />
          </>
        ) : (
          <circle cx="305" cy="150" r="24" stroke="var(--line-hi)" strokeOpacity={alpha(0.46, boost)} strokeWidth={1.1 * sw} />
        )}
        {PEAK_RAYS.map((ray, index) => (
          <line
            key={index}
            x1={ray.x1}
            y1={ray.y1}
            x2={ray.x2}
            y2={ray.y2}
            stroke="var(--line-a)"
            strokeOpacity={alpha(0.28, boost)}
            strokeWidth={0.85 * sw}
          />
        ))}
      </g>

      {/* Fine silk cloud filaments */}
      <g fill="none" strokeLinecap="round" className={cn(animate && 'now-ambient-drift')}>
        <path d="M180,224 C290,213 425,216 555,222" stroke={`url(#${uid}-filament)`} strokeWidth={0.95 * sw} />
        <path d="M475,146 C565,137 665,139 755,146" stroke={`url(#${uid}-filament)`} strokeWidth={0.85 * sw} />
      </g>
      <g fill="none" strokeLinecap="round" className={cn(animate && 'now-ambient-drift-slow')}>
        <path d="M715,172 C840,160 975,163 1110,171" stroke={`url(#${uid}-filament)`} strokeWidth={0.9 * sw} />
      </g>

      {/* Distant bird strokes */}
      <g fill="none" strokeLinecap="round" strokeLinejoin="round" className={palette.inkStroke} strokeOpacity={alpha(0.28, boost)} strokeWidth={1.15 * sw}>
        <path d="M750,334 Q759,326 767,333 Q775,327 784,335" />
        <path d="M794,320 Q800,315 806,319 Q812,315 818,320" />
      </g>

      {/* 14-line topographic ridge contour family (far sparse -> near dense) */}
      <g fill="none" strokeLinecap="round" className={cn(animate && 'now-ambient-drift-slow')}>
        {PEAK_CONTOURS.filter((c) => c.tier === 'far').map((contour, index) => (
          <path
            key={index}
            d={contour.d}
            stroke={`url(#${uid}-contour-far)`}
            strokeWidth={contour.weight * sw}
            strokeOpacity={contour.alphaScale}
          />
        ))}
      </g>
      <g fill="none" strokeLinecap="round" className={cn(animate && 'now-ambient-drift')}>
        {PEAK_CONTOURS.filter((c) => c.tier !== 'far').map((contour, index) => (
          <path
            key={index}
            d={contour.d}
            stroke={`url(#${uid}-${contour.tier === 'mid' ? 'contour-mid' : 'contour-near'})`}
            strokeWidth={contour.weight * sw}
            strokeOpacity={contour.alphaScale}
          />
        ))}
      </g>
    </svg>
  )
}

/** Generates subtle undulating perspective wave path across x in [-120, 1320]. */
function buildWavePath(y: number, index: number, total: number): string {
  const t = index / (total - 1)
  const amp = 0.7 + t * 3.1
  const phase = index * 0.85
  const xs = [-120, 60, 240, 420, 600, 780, 960, 1140, 1320]
  const pts = xs.map((x) => {
    const wy = y + Math.sin(x * 0.0085 + phase) * amp + Math.cos(x * 0.015 - phase * 0.6) * (amp * 0.35)
    return { x, y: Number(wy.toFixed(1)) }
  })
  let d = `M${pts[0].x},${pts[0].y}`
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[Math.max(0, i - 1)]
    const p1 = pts[i]
    const p2 = pts[i + 1]
    const p3 = pts[Math.min(pts.length - 1, i + 2)]
    const c1x = Number((p1.x + (p2.x - p0.x) / 6).toFixed(1))
    const c1y = Number((p1.y + (p2.y - p0.y) / 6).toFixed(1))
    const c2x = Number((p2.x - (p3.x - p1.x) / 6).toFixed(1))
    const c2y = Number((p2.y - (p3.y - p1.y) / 6).toFixed(1))
    d += ` C${c1x},${c1y} ${c2x},${c2y} ${p2.x},${p2.y}`
  }
  return d
}

// 16 perspective sea rows: dense right under y=505 horizon, progressively wider toward foreground.
const SEA_WAVE_YS = [513, 518, 524, 531, 539, 548, 558, 570, 583, 598, 615, 634, 655, 678, 703, 730]
const SEA_WAVES = SEA_WAVE_YS.map((y, index) => ({
  y,
  even: index % 2 === 0,
  d: buildWavePath(y, index, SEA_WAVE_YS.length),
  weight: 0.72 + (index / (SEA_WAVE_YS.length - 1)) * 0.42,
  opacity: 0.92 - (index / (SEA_WAVE_YS.length - 1)) * 0.32,
}))

/**
 * 2. Sea & Horizon: 16 perspective wave streamlines (dense near horizon, wide in foreground)
 * with luminous central specular glint, slender horizon line and open celestial arcs.
 */
function SeaScene({ uid, palette, night, animate, boost, preview }: InnerProps) {
  const sw = preview ? 1.6 : 1
  return (
    <svg
      data-slot="now-ambient-svg"
      data-scene="sea"
      viewBox={preview ? '-30 170 1260 515' : '0 0 1200 720'}
      preserveAspectRatio={preview ? 'xMidYMid slice' : 'xMidYMax slice'}
      className={cn('absolute inset-0 size-full', palette.varsClass)}
    >
      <defs>
        <radialGradient id={`${uid}-sun-air`} cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="var(--line-glow)" stopOpacity={alpha(0.12, boost)} />
          <stop offset="55%" stopColor="var(--line-glow)" stopOpacity={alpha(0.04, boost)} />
          <stop offset="100%" stopColor="var(--line-glow)" stopOpacity="0" />
        </radialGradient>
        <linearGradient id={`${uid}-horizon-line`} gradientUnits="userSpaceOnUse" x1="-140" y1="505" x2="1340" y2="505">
          <stop offset="0%" stopColor="var(--line-b)" stopOpacity="0" />
          <stop offset="22%" stopColor="var(--line-a)" stopOpacity={alpha(0.28, boost)} />
          <stop offset="55%" stopColor="var(--line-a)" stopOpacity={alpha(0.36, boost)} />
          <stop offset="72%" stopColor="var(--line-hi)" stopOpacity={alpha(0.68, boost)} />
          <stop offset="88%" stopColor="var(--line-a)" stopOpacity={alpha(0.38, boost)} />
          <stop offset="100%" stopColor="var(--line-b)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-wave-stroke`} x1="0" y1="0.5" x2="1" y2="0.5">
          <stop offset="0%" stopColor="var(--line-b)" stopOpacity="0" />
          <stop offset="22%" stopColor="var(--line-a)" stopOpacity={alpha(0.18, boost)} />
          <stop offset="52%" stopColor="var(--line-a)" stopOpacity={alpha(0.22, boost)} />
          <stop offset="68%" stopColor="var(--line-hi)" stopOpacity={alpha(0.56, boost)} />
          <stop offset="78%" stopColor="var(--line-a)" stopOpacity={alpha(0.26, boost)} />
          <stop offset="100%" stopColor="var(--line-b)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-wave-ink`} x1="0" y1="0.5" x2="1" y2="0.5">
          <stop offset="0%" stopColor="var(--line-ink)" stopOpacity="0" />
          <stop offset="26%" stopColor="var(--line-ink)" stopOpacity={alpha(0.15, boost)} />
          <stop offset="64%" stopColor="var(--line-hi)" stopOpacity={alpha(0.42, boost)} />
          <stop offset="76%" stopColor="var(--line-ink)" stopOpacity={alpha(0.22, boost)} />
          <stop offset="100%" stopColor="var(--line-ink)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-filament`} x1="0" y1="0.5" x2="1" y2="0.5">
          <stop offset="0%" stopColor="var(--line-b)" stopOpacity="0" />
          <stop offset="35%" stopColor="var(--line-a)" stopOpacity={alpha(0.22, boost)} />
          <stop offset="75%" stopColor="var(--line-hi)" stopOpacity={alpha(0.24, boost)} />
          <stop offset="100%" stopColor="var(--line-b)" stopOpacity="0" />
        </linearGradient>
      </defs>

      {/* Single soft horizon sun/moon air bloom */}
      <circle cx="855" cy="482" r="280" fill={`url(#${uid}-sun-air)`} />

      {/* Open celestial concentric arcs resting cleanly on the sea horizon y=505 */}
      <g
        fill="none"
        strokeLinecap="round"
        className={cn(animate && 'now-ambient-breathe')}
        style={{ transformOrigin: '855px 468px' }}
      >
        <path d="M732.5,505 A128,128 0 1,1 977.5,505" stroke="var(--line-b)" strokeOpacity={alpha(0.12, boost)} strokeWidth={0.75 * sw} />
        <path d="M770.8,505 A92,92 0 1,1 939.2,505" stroke="var(--line-a)" strokeOpacity={alpha(0.22, boost)} strokeWidth={0.85 * sw} strokeDasharray="3 7" />
        <path d="M802.8,505 A64,64 0 1,1 907.2,505" stroke="var(--line-hi)" strokeOpacity={alpha(0.3, boost)} strokeWidth={0.95 * sw} />
        {night ? (
          <>
            <path
              d="M862,442 A28,28 0 1,0 879,486 A23,23 0 1,1 862,442 Z"
              stroke="var(--line-hi)"
              strokeOpacity={alpha(0.56, boost)}
              strokeWidth={1.15 * sw}
              strokeLinejoin="round"
            />
            <path
              d="M862,442 A28,28 0 0,1 879,486"
              stroke="var(--line-a)"
              strokeOpacity={alpha(0.2, boost)}
              strokeWidth={0.75 * sw}
              strokeDasharray="1.5 3.5"
            />
          </>
        ) : (
          <path d="M835.1,505 A42,42 0 1,1 874.9,505" stroke="var(--line-hi)" strokeOpacity={alpha(0.52, boost)} strokeWidth={1.15 * sw} />
        )}
      </g>

      {/* Fine sky cirrus filaments & distant headland contours */}
      <g fill="none" strokeLinecap="round" className={cn(animate && 'now-ambient-drift-slow')}>
        <path d="M135,256 C260,246 395,248 525,255" stroke={`url(#${uid}-filament)`} strokeWidth={0.85 * sw} />
        <path d="M620,316 C745,307 885,308 1010,316" stroke={`url(#${uid}-filament)`} strokeWidth={0.85 * sw} />
      </g>
      <g fill="none" strokeLinecap="round" strokeWidth={0.9 * sw}>
        <path d="M-120,486 C15,480 140,488 310,505" stroke={`url(#${uid}-filament)`} strokeOpacity={0.85} />
        <path d="M1010,505 C1110,492 1210,485 1340,491" stroke={`url(#${uid}-filament)`} strokeOpacity={0.85} />
      </g>

      {/* Slender luminous sea horizon */}
      <line x1="-140" y1="505" x2="1340" y2="505" stroke={`url(#${uid}-horizon-line)`} strokeWidth={1.15 * sw} />

      {/* 16 perspective wave streamlines: dense near horizon, spreading wider downward */}
      <g fill="none" strokeLinecap="round" className={cn(animate && 'now-ambient-shimmer')} style={{ transformOrigin: '855px 575px' }}>
        {SEA_WAVES.filter((w) => w.even).map((wave, index) => (
          <path
            key={index}
            d={wave.d}
            stroke={`url(#${uid}-wave-stroke)`}
            strokeWidth={wave.weight * sw}
            strokeOpacity={wave.opacity}
          />
        ))}
      </g>
      <g fill="none" strokeLinecap="round" className={cn(animate && 'now-ambient-ripple')} style={{ transformOrigin: '855px 595px' }}>
        {SEA_WAVES.filter((w) => !w.even).map((wave, index) => (
          <path
            key={index}
            d={wave.d}
            stroke={`url(#${uid}-wave-ink)`}
            strokeWidth={wave.weight * sw}
            strokeOpacity={wave.opacity}
          />
        ))}
      </g>
    </svg>
  )
}

// 14 parallel perspective sunbeam rays divided cleanly into two casement bundles.
const WINDOW_RAY_X_TOP = [696, 730, 764, 798, 832, 866, 928, 964, 1000, 1036, 1072, 1108, 1158, 1196]
const WINDOW_RAYS = WINDOW_RAY_X_TOP.map((topX, index) => {
  // Subtle perspective divergence (~3%) from distant window toward floor.
  const shift = -470 - (index - 6) * 8
  return {
    x1: topX,
    y1: -40,
    x2: topX + shift,
    y2: 750,
    bright: index >= 6 && index <= 11,
    secondary: index >= 12,
  }
})

function lineIntersect(
  x1: number, y1: number, x2: number, y2: number,
  x3: number, y3: number, x4: number, y4: number,
) {
  const denom = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4)
  const px = ((x1 * y2 - y1 * x2) * (x3 - x4) - (x1 - x2) * (x3 * y4 - y3 * x4)) / denom
  const py = ((x1 * y2 - y1 * x2) * (y3 - y4) - (y1 - y2) * (x3 * y4 - y3 * x4)) / denom
  return { x: Number(px.toFixed(1)), y: Number(py.toFixed(1)) }
}

const WINDOW_FRAME_RAILS: [number, number, number, number][] = [
  [680, -40, 210, 750],
  [882, -40, 400, 750],
  [916, -40, 444, 750],
  [1122, -40, 672, 750],
]
const WINDOW_TRANSOMS: [number, number, number, number][] = [
  [435, 248, 1175, 126],
  [415, 281, 1158, 158],
]
const WINDOW_CROSSHAIRS = [
  lineIntersect(...WINDOW_FRAME_RAILS[0], ...WINDOW_TRANSOMS[0]),
  lineIntersect(...WINDOW_FRAME_RAILS[1], ...WINDOW_TRANSOMS[0]),
  lineIntersect(...WINDOW_FRAME_RAILS[1], ...WINDOW_TRANSOMS[1]),
  lineIntersect(...WINDOW_FRAME_RAILS[2], ...WINDOW_TRANSOMS[0]),
  lineIntersect(...WINDOW_FRAME_RAILS[2], ...WINDOW_TRANSOMS[1]),
  lineIntersect(...WINDOW_FRAME_RAILS[3], ...WINDOW_TRANSOMS[0]),
]

const WINDOW_MOTES: { cx: number; cy: number; r: number; hollow: boolean; delay: string }[] = [
  { cx: 682, cy: 138, r: 3.4, hollow: true, delay: '0s' },
  { cx: 794, cy: 112, r: 1.5, hollow: false, delay: '-4.2s' },
  { cx: 848, cy: 218, r: 3.8, hollow: true, delay: '-8.6s' },
  { cx: 615, cy: 274, r: 1.7, hollow: false, delay: '-2.5s' },
  { cx: 742, cy: 298, r: 3.0, hollow: true, delay: '-11.4s' },
  { cx: 522, cy: 388, r: 1.6, hollow: false, delay: '-6.8s' },
  { cx: 658, cy: 368, r: 1.4, hollow: false, delay: '-14.2s' },
]

/**
 * 3. Windowlight: architectural perspective ray array, fine double-hairline casement
 * mullions with `+` registration marks, pure-stroke botanical branch and hollow motes.
 */
function WindowScene({ uid, palette, animate, boost, preview }: InnerProps) {
  const sw = preview ? 1.55 : 1
  return (
    <svg
      data-slot="now-ambient-svg"
      data-scene="window"
      viewBox={preview ? '40 0 1170 495' : '0 0 1200 720'}
      preserveAspectRatio="xMidYMin slice"
      className={cn('absolute inset-0 size-full', palette.varsClass)}
    >
      <defs>
        <radialGradient id={`${uid}-source-air`} cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="var(--line-glow)" stopOpacity={alpha(0.11, boost)} />
          <stop offset="55%" stopColor="var(--line-glow)" stopOpacity={alpha(0.035, boost)} />
          <stop offset="100%" stopColor="var(--line-glow)" stopOpacity="0" />
        </radialGradient>
        <linearGradient id={`${uid}-ray`} x1="0.82" y1="0" x2="0.18" y2="1">
          <stop offset="0%" stopColor="var(--line-hi)" stopOpacity={alpha(0.32, boost)} />
          <stop offset="48%" stopColor="var(--line-a)" stopOpacity={alpha(0.15, boost)} />
          <stop offset="100%" stopColor="var(--line-a)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-ray-soft`} x1="0.82" y1="0" x2="0.18" y2="1">
          <stop offset="0%" stopColor="var(--line-b)" stopOpacity={alpha(0.2, boost)} />
          <stop offset="55%" stopColor="var(--line-b)" stopOpacity={alpha(0.08, boost)} />
          <stop offset="100%" stopColor="var(--line-b)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-mullion`} x1="0.8" y1="0" x2="0.2" y2="1">
          <stop offset="0%" stopColor="var(--line-ink)" stopOpacity={alpha(0.36, boost)} />
          <stop offset="55%" stopColor="var(--line-a)" stopOpacity={alpha(0.22, boost)} />
          <stop offset="100%" stopColor="var(--line-ink)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-transom`} x1="0" y1="0.5" x2="1" y2="0.2">
          <stop offset="0%" stopColor="var(--line-ink)" stopOpacity="0" />
          <stop offset="22%" stopColor="var(--line-ink)" stopOpacity={alpha(0.32, boost)} />
          <stop offset="78%" stopColor="var(--line-hi)" stopOpacity={alpha(0.4, boost)} />
          <stop offset="100%" stopColor="var(--line-ink)" stopOpacity={alpha(0.12, boost)} />
        </linearGradient>
      </defs>

      {/* Single soft corner air warmth */}
      <circle cx="1015" cy="40" r="390" fill={`url(#${uid}-source-air)`} />

      {/* Slanted perspective sunbeam hairline array + double-hairline casement frame */}
      <g fill="none" strokeLinecap="round" className={cn(animate && 'now-ambient-drift')}>
        {WINDOW_RAYS.map((ray, index) => (
          <line
            key={index}
            x1={ray.x1}
            y1={ray.y1}
            x2={ray.x2}
            y2={ray.y2}
            stroke={`url(#${uid}-${ray.secondary ? 'ray-soft' : 'ray'})`}
            strokeWidth={(ray.bright ? 0.95 : 0.78) * sw}
            strokeOpacity={ray.secondary ? 0.75 : 1}
          />
        ))}

        {/* Perspective vertical mullions & outer frame rails */}
        <line x1="680" y1="-40" x2="210" y2="750" stroke={`url(#${uid}-mullion)`} strokeWidth={1.1 * sw} />
        <line x1="882" y1="-40" x2="400" y2="750" stroke={`url(#${uid}-mullion)`} strokeWidth={1.05 * sw} />
        <line x1="916" y1="-40" x2="444" y2="750" stroke={`url(#${uid}-mullion)`} strokeWidth={1.05 * sw} />
        <line x1="1122" y1="-40" x2="672" y2="750" stroke={`url(#${uid}-mullion)`} strokeWidth={1.1 * sw} />

        {/* Perspective horizontal transom crossbars */}
        <line x1="435" y1="248" x2="1175" y2="126" stroke={`url(#${uid}-transom)`} strokeWidth={1.05 * sw} />
        <line x1="415" y1="281" x2="1158" y2="158" stroke={`url(#${uid}-transom)`} strokeWidth={1.05 * sw} />

        {/* Micro architectural `+` registration crosshairs at pane junctions */}
        <g className={palette.inkStroke} strokeOpacity={alpha(0.38, boost)} strokeWidth={0.9 * sw}>
          {WINDOW_CROSSHAIRS.map((pt, index) => {
            const arm = 5.5 * (preview ? 1.25 : 1)
            return (
              <path
                key={index}
                d={`M${pt.x - arm},${pt.y} h${arm * 2} M${pt.x},${pt.y - arm} v${arm * 2}`}
              />
            )
          })}
        </g>
      </g>

      {/* Pure fine-line botanical branch silhouette outside upper-right window */}
      <g
        fill="none"
        strokeLinecap="round"
        strokeLinejoin="round"
        className={cn(palette.inkStroke, animate && 'now-ambient-sway')}
        style={{ transformOrigin: '1215px -15px' }}
      >
        <path d="M1222,-18 C1170,36 1110,94 1030,180" strokeOpacity={alpha(0.32, boost)} strokeWidth={1.35 * sw} />
        <path d="M1142,56 C1100,48 1064,46 1028,58" strokeOpacity={alpha(0.28, boost)} strokeWidth={1.05 * sw} />
        <path d="M1088,118 C1050,124 1020,136 992,158" strokeOpacity={alpha(0.28, boost)} strokeWidth={1.05 * sw} />

        {/* Hollow leaf contours + delicate midribs */}
        <g strokeOpacity={alpha(0.3, boost)} strokeWidth={0.85 * sw}>
          <path d="M1176,22 C1154,10 1134,12 1126,28 C1146,35 1166,34 1176,22 Z" />
          <path d="M1146,54 C1136,28 1118,16 1102,24 C1110,44 1128,55 1146,54 Z" />
          <path d="M1104,49 C1084,29 1064,26 1052,39 C1070,51 1089,54 1104,49 Z" />
          <path d="M1066,50 C1046,40 1025,44 1016,58 C1034,64 1054,60 1066,50 Z" />
          <path d="M1144,58 C1156,82 1148,104 1132,110 C1126,90 1130,70 1144,58 Z" />
          <path d="M1112,90 C1090,78 1068,82 1058,96 C1078,104 1098,102 1112,90 Z" />
          <path d="M1086,120 C1098,144 1090,166 1074,172 C1068,152 1072,134 1086,120 Z" />
          <path d="M1044,132 C1020,118 996,122 986,138 C1008,146 1028,144 1044,132 Z" />
          <path d="M1028,176 C1006,184 994,202 998,218 C1016,210 1028,194 1028,176 Z" />
        </g>
      </g>

      {/* Hollow light rings & floating micro-motes inside the sunbeams */}
      <g>
        {WINDOW_MOTES.map((mote, index) => (
          <circle
            key={index}
            cx={mote.cx}
            cy={mote.cy}
            r={mote.r * (preview ? 1.25 : 1)}
            fill={mote.hollow ? 'none' : 'var(--line-hi)'}
            fillOpacity={mote.hollow ? undefined : alpha(0.42, boost)}
            stroke={mote.hollow ? 'var(--line-hi)' : undefined}
            strokeOpacity={mote.hollow ? alpha(0.36, boost) : undefined}
            strokeWidth={mote.hollow ? 0.85 * sw : undefined}
            style={{ animationDelay: mote.delay }}
            className={cn(animate && 'now-ambient-float')}
          />
        ))}
      </g>
    </svg>
  )
}

const ORBIT_STARS: { cx: number; cy: number; r: number; hollow: boolean; delay: string }[] = [
  { cx: 118, cy: 108, r: 1.5, hollow: false, delay: '0s' },
  { cx: 282, cy: 78, r: 2.1, hollow: true, delay: '-1.8s' },
  { cx: 472, cy: 158, r: 1.4, hollow: false, delay: '-3.1s' },
  { cx: 562, cy: 76, r: 1.2, hollow: false, delay: '-0.9s' },
  { cx: 742, cy: 94, r: 2.3, hollow: true, delay: '-4.2s' },
  { cx: 1042, cy: 84, r: 1.5, hollow: false, delay: '-1.4s' },
  { cx: 1126, cy: 184, r: 1.6, hollow: false, delay: '-2.7s' },
  { cx: 184, cy: 292, r: 1.3, hollow: false, delay: '-3.8s' },
  { cx: 1028, cy: 368, r: 2.0, hollow: true, delay: '-0.5s' },
  { cx: 786, cy: 356, r: 1.2, hollow: false, delay: '-2.2s' },
]

const CONSTELLATION_NODES: { cx: number; cy: number }[] = [
  { cx: 155, cy: 175 },
  { cx: 315, cy: 148 },
  { cx: 338, cy: 222 },
  { cx: 252, cy: 268 },
]

const SPARKLE_STARS: { cx: number; cy: number; size: number; delay: string }[] = [
  { cx: 228, cy: 122, size: 8.5, delay: '-0.6s' },
  { cx: 392, cy: 106, size: 7, delay: '-2.8s' },
  { cx: 618, cy: 92, size: 9.5, delay: '-1.5s' },
  { cx: 1112, cy: 312, size: 7.5, delay: '-3.9s' },
]

const COMPANION_PLANET = ellipsePoint(895, 200, 322, 102, -22, 2.58)
const SECONDARY_NODE = ellipsePoint(895, 200, 244, 76, 15, -0.52)

/**
 * 4. Celestial Orbit: precision astrolabe linework—concentric graduated rings,
 * crosshairs, tilted elliptical orbits, pure-stroke crescent with inner chord glow,
 * anchored constellation and wireframe four-point stars.
 */
function OrbitScene({ uid, palette, night, animate, boost, preview }: InnerProps) {
  const sw = preview ? 1.55 : 1
  return (
    <svg
      data-slot="now-ambient-svg"
      data-scene="orbit"
      viewBox={preview ? '30 15 1140 465' : '0 0 1200 720'}
      preserveAspectRatio="xMidYMin slice"
      className={cn('absolute inset-0 size-full', palette.varsClass)}
    >
      <defs>
        <radialGradient id={`${uid}-astrolabe-air`} cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="var(--line-glow)" stopOpacity={alpha(0.11, boost)} />
          <stop offset="55%" stopColor="var(--line-glow)" stopOpacity={alpha(0.035, boost)} />
          <stop offset="100%" stopColor="var(--line-glow)" stopOpacity="0" />
        </radialGradient>
        <linearGradient id={`${uid}-axis-h`} gradientUnits="userSpaceOnUse" x1="560" y1="200" x2="1220" y2="200">
          <stop offset="0%" stopColor="var(--line-ink)" stopOpacity="0" />
          <stop offset="30%" stopColor="var(--line-ink)" stopOpacity={alpha(0.24, boost)} />
          <stop offset="50%" stopColor="var(--line-hi)" stopOpacity={alpha(0.36, boost)} />
          <stop offset="70%" stopColor="var(--line-ink)" stopOpacity={alpha(0.24, boost)} />
          <stop offset="100%" stopColor="var(--line-ink)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`${uid}-axis-v`} gradientUnits="userSpaceOnUse" x1="895" y1="-20" x2="895" y2="540">
          <stop offset="0%" stopColor="var(--line-ink)" stopOpacity="0" />
          <stop offset="30%" stopColor="var(--line-ink)" stopOpacity={alpha(0.24, boost)} />
          <stop offset="50%" stopColor="var(--line-hi)" stopOpacity={alpha(0.36, boost)} />
          <stop offset="70%" stopColor="var(--line-ink)" stopOpacity={alpha(0.24, boost)} />
          <stop offset="100%" stopColor="var(--line-ink)" stopOpacity="0" />
        </linearGradient>
      </defs>

      {/* Single soft cosmic air glow */}
      <circle cx="895" cy="200" r="275" fill={`url(#${uid}-astrolabe-air)`} />

      {/* Precision astrolabe rings, graduation ticks & axis crosshairs */}
      <g fill="none">
        <circle cx="895" cy="200" r="68" stroke="var(--line-a)" strokeOpacity={alpha(0.2, boost)} strokeWidth={0.8 * sw} />
        <circle cx="895" cy="200" r="106" className={palette.inkStroke} strokeOpacity={alpha(0.28, boost)} strokeWidth={0.95 * sw} />
        <circle
          cx="895"
          cy="200"
          r="156"
          className={palette.inkStroke}
          strokeOpacity={alpha(0.38, boost)}
          strokeWidth={1.5 * sw}
          strokeDasharray="1.2 15.1"
          strokeLinecap="round"
        />
        <circle cx="895" cy="200" r="206" stroke="var(--line-b)" strokeOpacity={alpha(0.22, boost)} strokeWidth={0.85 * sw} strokeDasharray="5 8" strokeLinecap="round" />
        <circle
          cx="895"
          cy="200"
          r="264"
          strokeOpacity={alpha(0.3, boost)}
          strokeWidth={4.8 * sw}
          strokeDasharray="1 26.6"
          strokeLinecap="butt"
          style={{ transformOrigin: '895px 200px' }}
          className={cn(palette.inkStroke, animate && 'now-ambient-orbit-rock')}
        />
        <circle cx="895" cy="200" r="340" stroke="var(--line-a)" strokeOpacity={alpha(0.14, boost)} strokeWidth={0.8 * sw} />
        <circle cx="895" cy="200" r="445" className={palette.inkStroke} strokeOpacity={alpha(0.12, boost)} strokeWidth={0.85 * sw} />

        {/* Crosshair longitude/latitude axes (interrupted smoothly outside lunar core) */}
        <line x1="560" y1="200" x2="848" y2="200" stroke={`url(#${uid}-axis-h)`} strokeWidth={0.85 * sw} />
        <line x1="942" y1="200" x2="1220" y2="200" stroke={`url(#${uid}-axis-h)`} strokeWidth={0.85 * sw} />
        <line x1="895" y1="-20" x2="895" y2="152" stroke={`url(#${uid}-axis-v)`} strokeWidth={0.85 * sw} />
        <line x1="895" y1="248" x2="895" y2="540" stroke={`url(#${uid}-axis-v)`} strokeWidth={0.85 * sw} />

        {/* Diagonal 45-degree degree ticks on the 106px meridian ring */}
        <line x1="965" y1="130" x2="976" y2="119" className={palette.inkStroke} strokeOpacity={alpha(0.28, boost)} strokeWidth={0.85 * sw} />
        <line x1="825" y1="130" x2="814" y2="119" className={palette.inkStroke} strokeOpacity={alpha(0.28, boost)} strokeWidth={0.85 * sw} />
        <line x1="965" y1="270" x2="976" y2="281" className={palette.inkStroke} strokeOpacity={alpha(0.28, boost)} strokeWidth={0.85 * sw} />
        <line x1="825" y1="270" x2="814" y2="281" className={palette.inkStroke} strokeOpacity={alpha(0.28, boost)} strokeWidth={0.85 * sw} />

        {/* Tilted intersecting elliptical planetary orbits */}
        <ellipse
          cx="895"
          cy="200"
          rx="322"
          ry="102"
          transform="rotate(-22 895 200)"
          className={palette.inkStroke}
          strokeOpacity={alpha(0.26, boost)}
          strokeWidth={0.95 * sw}
        />
        <ellipse
          cx="895"
          cy="200"
          rx="244"
          ry="76"
          transform="rotate(15 895 200)"
          stroke="var(--line-a)"
          strokeOpacity={alpha(0.22, boost)}
          strokeWidth={0.85 * sw}
          strokeDasharray="3 7"
        />
      </g>

      {/* Hollow ringed companion planet & distant orbital node */}
      <g fill="none">
        <circle cx={COMPANION_PLANET.x} cy={COMPANION_PLANET.y} r={5 * (preview ? 1.15 : 1)} stroke="var(--line-hi)" strokeOpacity={alpha(0.54, boost)} strokeWidth={1.05 * sw} />
        <ellipse
          cx={COMPANION_PLANET.x}
          cy={COMPANION_PLANET.y}
          rx={11.5 * (preview ? 1.15 : 1)}
          ry={3.4 * (preview ? 1.15 : 1)}
          transform={`rotate(-22 ${COMPANION_PLANET.x} ${COMPANION_PLANET.y})`}
          className={palette.inkStroke}
          strokeOpacity={alpha(0.36, boost)}
          strokeWidth={0.85 * sw}
        />
        <circle cx={SECONDARY_NODE.x} cy={SECONDARY_NODE.y} r={7 * (preview ? 1.15 : 1)} className={palette.inkStroke} strokeOpacity={alpha(0.28, boost)} strokeWidth={0.8 * sw} />
        <circle cx={SECONDARY_NODE.x} cy={SECONDARY_NODE.y} r={2.2 * (preview ? 1.15 : 1)} stroke="var(--line-hi)" strokeOpacity={alpha(0.56, boost)} strokeWidth={0.9 * sw} />
      </g>

      {/* Pure-stroke crescent moon with inner chord highlight or engraved solar medallion */}
      <g
        fill="none"
        strokeLinecap="round"
        className={cn(animate && 'now-ambient-breathe')}
        style={{ transformOrigin: '895px 200px' }}
      >
        {night ? (
          <>
            <path
              d="M899.2,170.3 A30,30 0 1,0 922,213.2 A27,27 0 0,1 899.2,170.3 Z"
              stroke="var(--line-hi)"
              strokeOpacity={alpha(0.58, boost)}
              strokeWidth={1.2 * sw}
              strokeLinejoin="round"
            />
            <path
              d="M893,174 A25,25 0 0,0 874,213"
              stroke="var(--line-a)"
              strokeOpacity={alpha(0.28, boost)}
              strokeWidth={0.8 * sw}
            />
            <path
              d="M899.2,170.3 A30,30 0 0,1 922,213.2"
              stroke="var(--line-a)"
              strokeOpacity={alpha(0.2, boost)}
              strokeWidth={0.8 * sw}
              strokeDasharray="1.5 3.5"
            />
          </>
        ) : (
          <>
            <circle cx="895" cy="200" r="30" stroke="var(--line-hi)" strokeOpacity={alpha(0.52, boost)} strokeWidth={1.15 * sw} />
            <circle cx="895" cy="200" r="21" stroke="var(--line-a)" strokeOpacity={alpha(0.26, boost)} strokeWidth={0.8 * sw} strokeDasharray="2 4" />
            <path d="M890,200 h10 M895,195 v10" className={palette.inkStroke} strokeOpacity={alpha(0.32, boost)} strokeWidth={0.85 * sw} />
          </>
        )}
      </g>

      {/* Upper-left constellation hairline & hollow vertex nodes */}
      <path
        d="M155,175 L228,122 L315,148 L392,106 M315,148 L338,222 L252,268"
        fill="none"
        className={palette.inkStroke}
        strokeOpacity={alpha(0.22, boost)}
        strokeWidth={0.85 * sw}
        strokeDasharray="2.5 4.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <g fill="none">
        {CONSTELLATION_NODES.map((node, index) => (
          <circle
            key={index}
            cx={node.cx}
            cy={node.cy}
            r={2.3 * (preview ? 1.25 : 1)}
            className={palette.inkStroke}
            strokeOpacity={alpha(0.42, boost)}
            strokeWidth={0.9 * sw}
          />
        ))}
      </g>

      {/* Wireframe four-point sparkle stars */}
      <g>
        {SPARKLE_STARS.map((star, index) => (
          <g
            key={index}
            style={{ animationDelay: star.delay }}
            className={cn(animate && 'now-ambient-twinkle')}
          >
            <path
              d={fourPointStar(star.cx, star.cy, star.size * (preview ? 1.2 : 1))}
              fill="none"
              className={palette.inkStroke}
              strokeOpacity={alpha(0.52, boost)}
              strokeWidth={0.95 * sw}
              strokeLinejoin="round"
            />
            <circle cx={star.cx} cy={star.cy} r={1.1 * (preview ? 1.2 : 1)} fill="var(--line-hi)" fillOpacity={alpha(0.65, boost)} />
          </g>
        ))}
      </g>

      {/* Fine pinprick & hollow micro-stars */}
      <g>
        {ORBIT_STARS.map((star, index) => (
          <circle
            key={index}
            cx={star.cx}
            cy={star.cy}
            r={star.r * (preview ? 1.25 : 1)}
            fill={star.hollow ? 'none' : undefined}
            fillOpacity={star.hollow ? undefined : alpha(0.46, boost)}
            strokeOpacity={star.hollow ? alpha(0.42, boost) : undefined}
            strokeWidth={star.hollow ? 0.85 * sw : undefined}
            className={cn(star.hollow ? palette.inkStroke : palette.inkFill, animate && 'now-ambient-twinkle')}
            style={{ animationDelay: star.delay }}
          />
        ))}
      </g>
    </svg>
  )
}
