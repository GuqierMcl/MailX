import { t } from '@/i18n'
import type { ChatContext } from '@/generated/accounts'

/** What the suggestions may mention, all from data already on screen. Nothing is fetched or sent. */
export type SuggestionFacts = {
  /** The sender of the newest unread message in the list (list view), or of the open email. */
  sender?: string | null
  /** How many emails are selected. */
  count?: number
}

/** How many suggestions show at once. */
export const SUGGESTION_COUNT = 3
const MAX_NAME_CHARS = 20

type Candidate = { key: string; text: () => string; needs?: 'sender' }

const name = (facts: SuggestionFacts) => {
  const value = (facts.sender ?? '').replace(/\s+/g, ' ').trim()
  return value.length > MAX_NAME_CHARS ? `${value.slice(0, MAX_NAME_CHARS)}…` : value
}

/**
 * The pool for each kind of view (ai-chat-ui.md 5.1). Plain requests a user could type; anything that
 * changes mail goes through the assistant's usual permission and confirmation rules like any message.
 */
function pool(type: ChatContext['type'] | 'none', facts: SuggestionFacts): Candidate[] {
  const sender = name(facts)
  switch (type) {
    case 'email': return [
      { key: 'summarize', text: () => t('ai:suggest.email.summarize') },
      { key: 'todo', text: () => t('ai:suggest.email.todo') },
      { key: 'reply', text: () => t('ai:suggest.email.reply') },
      { key: 'deadline', text: () => t('ai:suggest.email.deadline') },
      { key: 'keyPoints', text: () => t('ai:suggest.email.keyPoints') },
      { key: 'explain', text: () => t('ai:suggest.email.explain') },
      { key: 'moreFrom', needs: 'sender', text: () => t('ai:suggest.email.moreFrom', { name: sender }) },
    ]
    case 'emails': return [
      { key: 'summarize', text: () => t('ai:suggest.emails.summarize', { count: facts.count ?? 0 }) },
      { key: 'needReply', text: () => t('ai:suggest.emails.needReply') },
      { key: 'todos', text: () => t('ai:suggest.emails.todos') },
      { key: 'common', text: () => t('ai:suggest.emails.common') },
      { key: 'archive', text: () => t('ai:suggest.emails.archive') },
      { key: 'markRead', text: () => t('ai:suggest.emails.markRead') },
    ]
    case 'draft': return [
      { key: 'review', text: () => t('ai:suggest.draft.review') },
      { key: 'formal', text: () => t('ai:suggest.draft.formal') },
      { key: 'shorter', text: () => t('ai:suggest.draft.shorter') },
      { key: 'missing', text: () => t('ai:suggest.draft.missing') },
      { key: 'subject', text: () => t('ai:suggest.draft.subject') },
      { key: 'typos', text: () => t('ai:suggest.draft.typos') },
    ]
    default: return [
      { key: 'latestFrom', needs: 'sender', text: () => t('ai:suggest.mail.latestFrom', { name: sender }) },
      { key: 'deadlines', text: () => t('ai:suggest.mail.deadlines') },
      { key: 'archiveOld', text: () => t('ai:suggest.mail.archiveOld') },
      { key: 'important', text: () => t('ai:suggest.mail.important') },
      { key: 'needReply', text: () => t('ai:suggest.mail.needReply') },
      { key: 'today', text: () => t('ai:suggest.mail.today') },
      { key: 'attachments', text: () => t('ai:suggest.mail.attachments') },
      { key: 'starred', text: () => t('ai:suggest.mail.starred') },
    ]
  }
}

/**
 * A few requests to offer above the capsule, picked at random from the pool for what is on screen,
 * so they are not the same every time. `random` returns [0, 1) like `Math.random`.
 */
export function capsuleSuggestions(
  context: ChatContext | null,
  facts: SuggestionFacts = {},
  random: () => number = Math.random,
): { key: string; text: string }[] {
  const usable = pool(context?.type ?? 'none', facts).filter((candidate) => candidate.needs !== 'sender' || name(facts) !== '')
  // A partial Fisher–Yates shuffle: the first few places are a uniform random pick.
  const picked = [...usable]
  const count = Math.min(SUGGESTION_COUNT, picked.length)
  for (let index = 0; index < count; index += 1) {
    const other = index + Math.floor(random() * (picked.length - index))
    ;[picked[index], picked[other]] = [picked[other], picked[index]]
  }
  return picked.slice(0, count).map((candidate) => ({ key: candidate.key, text: candidate.text() }))
}
