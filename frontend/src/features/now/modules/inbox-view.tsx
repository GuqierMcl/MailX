import { t } from '@/i18n'
import { cn } from '@/lib/utils'
import { useCountUp, type EntranceMode } from '../motion'
import type { NowRenderEnv } from './types'

const linkClass = 'rounded-md px-3 py-1 text-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring'

export function ViewAllUnread({ unread, onOpenUnread }: { unread: number; onOpenUnread: NowRenderEnv['actions']['onOpenUnread'] }) {
  return (
    <button type="button" onClick={onOpenUnread} className={cn(linkClass, 'self-start')}>
      {t('now:viewAllUnread', { count: unread })}
    </button>
  )
}

/** A small count that leads somewhere; screen readers get the label and the number once. */
export function StatButton({ value, label, mode, onClick }: { value: number; label: string; mode: EntranceMode; onClick: () => void }) {
  const shown = useCountUp(value, mode === 'first')
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={`${label} ${value}`}
      className="flex items-baseline gap-1.5 rounded-lg bg-muted/40 px-3 py-1.5 text-sm outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span className="text-muted-foreground">{label}</span>
      <span className="font-semibold tabular-nums">{shown}</span>
    </button>
  )
}
