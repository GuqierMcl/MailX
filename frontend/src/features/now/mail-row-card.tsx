import { t } from '@/i18n'
import type { EmailSummary } from '@/generated/accounts'
import { cn } from '@/lib/utils'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { displayName } from '@/features/mail/format'
import { relativeTime } from './relative-time'

/** A row of the Now page and of the assistant's cards: faint fill on hover, a ring for the keyboard, no border. */
export const rowClass = 'flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring'

/**
 * One mail as a row: sender avatar, sender, subject and how long ago. The Now page's unread list and the
 * mail cards in the assistant's replies share it. `isNew` adds the "new" tag; `unread` a dot before the time.
 */
export function MailRowCard({ email, language, now, isNew = false, unread = false, onOpen, label, className }: {
  email: EmailSummary
  language: string
  now: number
  isNew?: boolean
  unread?: boolean
  onOpen: (email: EmailSummary) => void
  /** The accessible name, when the visible words are not enough. */
  label?: string
  className?: string
}) {
  const subject = email.subject || t('mail:noSubject')
  return (
    <button type="button" aria-label={label} onClick={() => onOpen(email)} className={cn(rowClass, className)}>
      <SenderAvatar sender={email.from} accountId={email.accountId} size="sm" />
      {/* Side by side when there is room, stacked in a narrow window. */}
      <span className="flex min-w-0 flex-1 flex-col sm:flex-row sm:items-center sm:gap-3">
        <span className="truncate font-medium sm:w-32 sm:shrink-0">{displayName(email.from)}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{subject}</span>
      </span>
      {isNew && <span data-slot="now-new-tag" className="shrink-0 rounded-sm bg-foreground/5 px-1.5 py-px text-[11px] text-muted-foreground">{t('now:newTag')}</span>}
      {unread && <span data-slot="unread-dot" aria-hidden className="size-1.5 shrink-0 rounded-full bg-primary" />}
      <span className="shrink-0 text-xs text-muted-foreground">{relativeTime(email.receivedAt, language, now)}</span>
    </button>
  )
}
