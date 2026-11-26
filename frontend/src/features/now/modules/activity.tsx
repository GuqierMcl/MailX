import { t } from '@/i18n'
import type { OverviewActivity } from '@/generated/accounts'
import { ActivityView } from './activity-view'
import type { NowModule } from './types'

function hasActivity(activity: OverviewActivity | undefined): activity is OverviewActivity {
  return activity !== undefined
    && (activity.days.some((day) => day.received > 0) || activity.sentLastWeek + activity.sentPreviousWeek > 0)
}

/** The last two weeks: received and sent, the people in touch most, today. */
export const activityModule: NowModule = {
  id: 'activity',
  zone: 'review',
  hideable: true,
  fromOverview: true,
  title: () => t('now:recent'),
  score: () => 0,
  signature: ({ activity }) => activity
    ? [...activity.days.map((day) => day.received), activity.sentLastWeek, activity.sentPreviousWeek, activity.today.received, activity.today.unread].join('|')
    : '',
  hasContent: ({ activity }) => hasActivity(activity),
  render: ({ data, language, mode, contactAccountId, actions }) => data.activity && (
    <ActivityView activity={data.activity} language={language} mode={mode} contactAccountId={contactAccountId} onOpenContact={actions.onOpenContact} />
  ),
}
