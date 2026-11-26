import { t } from '@/i18n'
import { AttachmentsView } from './attachments-view'
import type { NowModule } from './types'

/**
 * Mail with attachments from the last week: not a thing to act on, so it comes last and shows as one
 * muted line that opens the list (now-page.md 4.3).
 */
export const attachmentsModule: NowModule = {
  id: 'attachments',
  zone: 'review',
  hideable: true,
  fromOverview: true,
  compact: true,
  title: () => t('now:attachments'),
  score: () => 90,
  signature: ({ attachments }) => attachments.map(({ email }) => email.id).join('|'),
  hasContent: ({ attachments }) => attachments.length > 0,
  render: ({ data, language, minute, actions }) => (
    <AttachmentsView attachments={data.attachments} language={language} minute={minute} onOpenEmail={actions.onOpenEmail} />
  ),
}
