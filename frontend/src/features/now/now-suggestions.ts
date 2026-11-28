import { t } from '@/i18n'
import type { NowData } from './modules/types'

export type NowSuggestion = { key: string; text: string }

const MAX_SUGGESTIONS = 3

/**
 * Local, deterministic prompt suggestions for the Now page's AI input, tailored
 * to what is on screen right now. Nothing is fetched or sent until clicked.
 */
export function nowSuggestions(data: NowData): NowSuggestion[] {
  const list: NowSuggestion[] = []
  if (data.inbox.newMail > 0) {
    list.push({ key: 'newMail', text: t('now:suggestNewMail') })
  }
  if (data.inbox.unread > 0) {
    list.push({ key: 'needReply', text: t('now:suggestNeedReply') })
  }
  if (data.inbox.starredUnread > 0) {
    list.push({ key: 'starred', text: t('now:suggestStarred') })
  }
  if (data.unreadSenders.length > 0) {
    list.push({ key: 'cleanup', text: t('now:suggestCleanup') })
  }
  if (data.attachments.length > 0) {
    list.push({ key: 'attachments', text: t('now:suggestAttachments') })
  }
  if (list.length < MAX_SUGGESTIONS) {
    list.push({ key: 'today', text: t('now:suggestToday') })
  }
  if (list.length < MAX_SUGGESTIONS) {
    list.push({ key: 'deadlines', text: t('now:suggestDeadlines') })
  }
  return list.slice(0, MAX_SUGGESTIONS)
}
