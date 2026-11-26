import { t } from '@/i18n'
import type { OverviewActivity } from '@/generated/accounts'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { compareWeeks } from '../activity'
import { ActivityChart } from '../activity-chart'
import type { EntranceMode } from '../motion'
import type { NowRenderEnv } from './types'

/** The last two weeks: the daily chart, then a few plain sentences. */
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
  return (
    <div className="flex flex-col gap-4 px-3">
      <ActivityChart days={activity.days} language={language} mode={mode} />
      <div data-slot="now-activity-lines" className="flex flex-col gap-1.5 text-sm text-muted-foreground">
        <p>{t(`now:receivedWeek.${received.kind}`, { count: activity.receivedLastWeek, diff: 'diff' in received ? received.diff : 0 })}</p>
        {showSent && <p>{t(`now:sentWeek.${sent.kind}`, { count: activity.sentLastWeek, diff: 'diff' in sent ? sent.diff : 0 })}</p>}
        {activity.topContacts.length > 0 && (
          <div className="flex flex-wrap items-center gap-x-1 gap-y-1">
            <span className="mr-1">{t('now:topContacts')}</span>
            {activity.topContacts.map((contact) => (
              <button
                key={contact.address}
                type="button"
                onClick={() => onOpenContact(contact)}
                className="flex max-w-48 items-center gap-1.5 rounded-full py-0.5 pr-2.5 pl-0.5 text-foreground outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring"
              >
                <SenderAvatar sender={{ name: contact.name, address: contact.address }} accountId={contactAccountId} size="sm" className="size-5 text-[9px]" />
                <span className="truncate">{contact.name?.trim() || contact.address}</span>
              </button>
            ))}
          </div>
        )}
        <p>
          {today.received === 0 ? t('now:todayNone')
            : today.unread > 0 ? t('now:todayUnread', { count: today.received, unread: today.unread })
              : t('now:todayAllRead', { count: today.received })}
        </p>
      </div>
    </div>
  )
}
