import { t } from '@/i18n'
import type { OverviewActivity } from '@/generated/accounts'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { cn } from '@/lib/utils'
import { compareWeeks } from '../activity'
import { ActivityChart } from '../activity-chart'
import { useCountUp, type EntranceMode } from '../motion'
import type { NowRenderEnv } from './types'

function Metric({ label, value, mode, delta }: {
  label: string
  value: number
  mode: EntranceMode
  delta?: { sign: '+' | '-'; diff: number } | null
}) {
  const shown = useCountUp(value, mode === 'first')
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      <div className="flex items-baseline gap-1.5">
        <span className="text-2xl font-semibold tracking-tight tabular-nums text-foreground">{shown}</span>
        {delta && (
          <span
            className={cn(
              'rounded-full px-1.5 py-px text-[11px] font-medium tabular-nums',
              delta.sign === '+'
                ? 'bg-primary/10 text-primary'
                : 'bg-foreground/6 text-muted-foreground',
            )}
          >
            {delta.sign}{delta.diff}
          </span>
        )}
      </div>
    </div>
  )
}

/** The last two weeks: soft surface with the daily chart on the left and KPI strip + details on the right on wide screens. */
export function ActivityView({ activity, language, mode, contactAccountId, onOpenContact }: {
  activity: OverviewActivity
  language: string
  mode: EntranceMode
  /** Any account; avatars are looked up by address. */
  contactAccountId: string
  onOpenContact: NowRenderEnv['actions']['onOpenContact']
}) {
  const complete = activity.previousWeekComplete
  const received = compareWeeks(activity.receivedLastWeek, activity.receivedPreviousWeek, complete)
  const sent = compareWeeks(activity.sentLastWeek, activity.sentPreviousWeek, complete)
  const showSent = activity.sentLastWeek + activity.sentPreviousWeek > 0
  const { today } = activity
  const receivedDelta = received.kind === 'more' ? { sign: '+' as const, diff: received.diff }
    : received.kind === 'fewer' ? { sign: '-' as const, diff: received.diff } : null
  const sentDelta = sent.kind === 'more' ? { sign: '+' as const, diff: sent.diff }
    : sent.kind === 'fewer' ? { sign: '-' as const, diff: sent.diff } : null

  return (
    <div data-slot="now-activity-card" className="grid grid-cols-1 gap-6 rounded-2xl bg-muted/50 p-4 sm:p-5 md:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)] md:items-end">
      <div className="flex flex-col gap-4">
        <div aria-hidden className="flex flex-wrap items-end gap-x-6 gap-y-2">
          <Metric label={t('now:metricReceived7d')} value={activity.receivedLastWeek} mode={mode} delta={receivedDelta} />
          {showSent && <Metric label={t('now:metricSent7d')} value={activity.sentLastWeek} mode={mode} delta={sentDelta} />}
          <Metric label={t('now:metricToday')} value={today.received} mode={mode} />
        </div>
        <ActivityChart days={activity.days} language={language} mode={mode} />
      </div>

      <div data-slot="now-activity-lines" className="flex flex-col gap-2 text-sm text-muted-foreground">
        <p>{t(`now:receivedWeek.${received.kind}`, { count: activity.receivedLastWeek, diff: 'diff' in received ? received.diff : 0 })}</p>
        {showSent && <p>{t(`now:sentWeek.${sent.kind}`, { count: activity.sentLastWeek, diff: 'diff' in sent ? sent.diff : 0 })}</p>}
        <p>
          {today.received === 0 ? t('now:todayNone')
            : today.unread > 0 ? t('now:todayUnread', { count: today.received, unread: today.unread })
              : t('now:todayAllRead', { count: today.received })}
        </p>
        {activity.topContacts.length > 0 && (
          <div className="flex flex-wrap items-center gap-x-1 gap-y-1 pt-0.5">
            <span className="mr-1">{t('now:topContacts')}</span>
            {activity.topContacts.map((contact) => (
              <button
                key={contact.address}
                type="button"
                onClick={() => onOpenContact(contact)}
                className="flex max-w-48 items-center gap-1.5 rounded-full bg-background/60 py-0.5 pr-2.5 pl-0.5 text-foreground outline-none hover:bg-background focus-visible:ring-2 focus-visible:ring-ring"
              >
                <SenderAvatar sender={{ name: contact.name, address: contact.address }} accountId={contactAccountId} size="sm" className="size-5 text-[9px]" />
                <span className="truncate">{contact.name?.trim() || contact.address}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
