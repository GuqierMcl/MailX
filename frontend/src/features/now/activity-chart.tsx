import { t } from '@/i18n'
import { useRef, useState, type KeyboardEvent } from 'react'
import type { DayActivity } from '@/generated/accounts'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import type { EntranceMode } from './motion'

/** Plot height in px (`h-14`); a day with mail is at least as tall as its rounded top. */
const HEIGHT = 56
const MIN_BAR = 4
const STUB = 2
const STAGGER_MS = 20
const RETURN_STAGGER_MS = 8

/** "11月3日 周一 · 12 封": the tooltip and the column's accessible name. */
function dayLabel(day: DayActivity, language: string): string {
  const date = new Date(day.start)
  return t('now:chartDay', {
    count: day.received,
    date: new Intl.DateTimeFormat(language, { month: 'short', day: 'numeric' }).format(date),
    weekday: new Intl.DateTimeFormat(language, { weekday: 'short' }).format(date),
  })
}

/**
 * Mail received per day over the last fourteen days, one series: today in the
 * accent, the other days in a quiet gray, no axes or gridlines. Each column is
 * a hit target the full height and width of its slot; one tab stop for the
 * chart, arrow keys move between days, and hover or focus shows the value.
 */
export function ActivityChart({ days, language, mode }: { days: DayActivity[]; language: string; mode: EntranceMode }) {
  const max = Math.max(1, ...days.map((day) => day.received))
  const [active, setActive] = useState(days.length - 1)
  const columns = useRef<(HTMLDivElement | null)[]>([])
  const move = (event: KeyboardEvent, index: number) => {
    const last = days.length - 1
    const next = event.key === 'ArrowLeft' ? Math.max(0, index - 1)
      : event.key === 'ArrowRight' ? Math.min(last, index + 1)
        : event.key === 'Home' ? 0 : event.key === 'End' ? last : null
    if (next === null) return
    event.preventDefault()
    setActive(next)
    columns.current[next]?.focus()
  }
  return (
    <div className="w-full max-w-91">
      <div role="group" aria-label={t('now:chartLabel')} className="flex h-14 items-end">
        {days.map((day, index) => {
          const today = index === days.length - 1
          const label = dayLabel(day, language)
          const height = day.received === 0 ? STUB : Math.max(MIN_BAR, Math.round((day.received / max) * HEIGHT))
          return (
            <Tooltip key={day.start}>
              <TooltipTrigger
                render={(
                  <div
                    ref={(element) => { columns.current[index] = element }}
                    role="img"
                    aria-label={label}
                    data-today={today || undefined}
                    tabIndex={index === active ? 0 : -1}
                    onFocus={() => setActive(index)}
                    onKeyDown={(event) => move(event, index)}
                    className="group flex h-full max-w-6.5 flex-1 items-end rounded-sm px-px outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  />
                )}
              >
                <span
                  aria-hidden
                  data-slot="activity-bar"
                  className={cn(
                    'block w-full origin-bottom',
                    day.received > 0 && 'rounded-t-[4px]',
                    today ? 'bg-primary' : 'bg-muted-foreground/30 group-hover:bg-muted-foreground/45',
                    mode === 'first' && 'animate-now-bar-grow',
                    mode === 'return' && 'animate-now-bar-grow-quick',
                  )}
                  style={{ height, animationDelay: mode === 'none' ? undefined : `${index * (mode === 'first' ? STAGGER_MS : RETURN_STAGGER_MS)}ms` }}
                />
              </TooltipTrigger>
              <TooltipContent>{label}</TooltipContent>
            </Tooltip>
          )
        })}
      </div>
      <div aria-hidden className="mt-1.5 flex justify-between px-px text-xs text-muted-foreground">
        <span>{t('now:chartStart')}</span>
        <span>{t('now:chartToday')}</span>
      </div>
    </div>
  )
}
