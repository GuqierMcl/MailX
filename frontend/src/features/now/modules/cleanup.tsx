import { t } from '@/i18n'
import { CleanupView } from './cleanup-view'
import type { NowModule } from './types'

/** Unread mail piled up from one sender: mark it read in one go, or look first. */
export const cleanupModule: NowModule = {
  id: 'cleanup',
  zone: 'flow',
  hideable: true,
  fromOverview: true,
  title: () => t('now:cleanupTitle'),
  score: () => 240,
  signature: ({ unreadSenders }) => unreadSenders.map((sender) => `${sender.address}:${sender.count}`).join('|'),
  hasContent: ({ unreadSenders }) => unreadSenders.length > 0,
  render: ({ data, client, contactAccountId, actions }) => (
    <CleanupView client={client} senders={data.unreadSenders} contactAccountId={contactAccountId} onOpenContact={actions.onOpenContact} />
  ),
}
