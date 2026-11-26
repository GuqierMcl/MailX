import { t } from '@/i18n'
import { AwayView } from './away-view'
import type { NowModule } from './types'

/** Away at least this long, with new mail, and the module shows. */
export const AWAY_MIN_MS = 3 * 3_600_000
const HOUR_MS = 3_600_000

/** A coarse "5 hours" / "3 days", in the interface language. */
export function awayDuration(ms: number): string {
  const hours = Math.floor(ms / HOUR_MS)
  return hours < 48 ? t('now:awayHours', { count: Math.max(1, hours) }) : t('now:awayDays', { count: Math.floor(hours / 24) })
}

/** Who wrote while the user was away (now-page.md 4.3). */
export const awayModule: NowModule = {
  id: 'away',
  zone: 'flow',
  hideable: true,
  fromOverview: true,
  title: () => t('now:awayTitle'),
  score: () => 290,
  signature: ({ away }) => `${away.sinceTime}|${away.total}`,
  hasContent: ({ away }) => away.ms >= AWAY_MIN_MS && away.total > 0,
  render: ({ data, contactAccountId, actions }) => (
    <AwayView away={data.away} contactAccountId={contactAccountId} onOpenContact={actions.onOpenContact} />
  ),
}
