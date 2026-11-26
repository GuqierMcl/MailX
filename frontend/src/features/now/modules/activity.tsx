import { t } from '@/i18n'
import type { OverviewActivity } from '@/generated/accounts'
import { ActivityView } from './activity-view'
import type { NowContext, NowModule } from './types'

function hasActivity(activity: OverviewActivity | undefined): activity is OverviewActivity {
  return activity !== undefined
    && (activity.days.some((day) => day.received > 0) || activity.sentLastWeek + activity.sentPreviousWeek > 0)
}

/** In the evening, on a Monday's first visit, or with nothing new in the inbox, a look back is in place. */
export const EVENING_HOUR = 18

function lookBackFits({ now, firstToday, data }: NowContext): boolean {
  const quiet = data.inbox.newMail === 0 && data.inbox.unread === 0
  return quiet || (now.getDay() === 1 && firstToday) || now.getHours() >= EVENING_HOUR
}

/** The last two weeks: received and sent, the people in touch most, today. */
export const activityModule: NowModule = {
  id: 'activity',
  zone: 'review',
  hideable: true,
  fromOverview: true,
  title: () => t('now:recent'),
  score: (context) => (lookBackFits(context) ? 230 : 100),
  signature: ({ activity }) => activity
    ? [...activity.days.map((day) => day.received), activity.sentLastWeek, activity.sentPreviousWeek, activity.today.received, activity.today.unread].join('|')
    : '',
  hasContent: ({ activity }) => hasActivity(activity),
  render: ({ data, language, mode, contactAccountId, actions }) => data.activity && (
    <ActivityView activity={data.activity} language={language} mode={mode} contactAccountId={contactAccountId} onOpenContact={actions.onOpenContact} />
  ),
}
