import { HugeiconsIcon } from '@hugeicons/react'
import { Archive02Icon, MailOpen01Icon, StarIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import type { EmailSummary } from '@/generated/accounts'
import { IconButton } from '@/components/icon-button'
import { cn } from '@/lib/utils'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { displayName } from '@/features/mail/format'
import { relativeTime } from './relative-time'

const rowBase = 'flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring'

/** A row of the Now page's lists: no fill until hovered, a ring for the keyboard, no border. */
export const rowClass = `${rowBase} hover:bg-foreground/5`

/**
 * The same row standing alone, as a card in an assistant reply: a faint fill at rest so it reads as one
 * thing among the text, a little deeper on hover and a slight press. Still no border (ui-design.md).
 */
export const cardClass = `${rowBase} bg-[color-mix(in_oklab,var(--muted)_65%,var(--background))] transition-[background-color,scale] duration-150 ease-out hover:bg-muted active:scale-[0.99] motion-reduce:transition-none motion-reduce:active:scale-100`

export type MailRowQuickActions = {
  onMarkRead: () => void
  onToggleStar: () => void
  onArchive?: () => void
  busy?: boolean
}

/**
 * One mail as a row: sender avatar, sender, subject and how long ago. The Now page's unread list and the
 * mail cards in the assistant's replies share it (`card` gives it the card fill). `isNew` adds the "new" tag; `unread` a dot before the time.
 * When `quickActions` is provided, hovering or focusing the row reveals inline star, mark-read and archive controls.
 */
export function MailRowCard({ email, language, now, isNew = false, unread = false, card = false, quickActions, onOpen, label, className }: {
  email: EmailSummary
  language: string
  now: number
  isNew?: boolean
  unread?: boolean
  /** Stands alone (an assistant reply) rather than in a list. */
  card?: boolean
  quickActions?: MailRowQuickActions
  onOpen: (email: EmailSummary) => void
  /** The accessible name, when the visible words are not enough. */
  label?: string
  className?: string
}) {
  const subject = email.subject || t('mail:noSubject')
  if (!quickActions) {
    return (
      <button type="button" aria-label={label} onClick={() => onOpen(email)} className={cn(card ? cardClass : rowClass, className)}>
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

  return (
    <div className={cn('group/row relative flex items-center rounded-lg transition-colors duration-150 hover:bg-foreground/5 focus-within:bg-foreground/5 motion-reduce:transition-none', className)}>
      <button
        type="button"
        aria-label={label}
        onClick={() => onOpen(email)}
        className={cn(rowBase, 'hover:bg-transparent')}
      >
        <SenderAvatar sender={email.from} accountId={email.accountId} size="sm" />
        <span className="flex min-w-0 flex-1 flex-col sm:flex-row sm:items-center sm:gap-3">
          <span className="truncate font-medium sm:w-32 sm:shrink-0">{displayName(email.from)}</span>
          <span className="flex min-w-0 flex-1 items-center gap-1.5">
            {email.isStarred && <HugeiconsIcon icon={StarIcon} aria-label={t('mail:starred')} fill="currentColor" className="size-3.5 shrink-0 text-amber-500" />}
            <span className="min-w-0 truncate text-muted-foreground">{subject}</span>
          </span>
        </span>
        <span className="flex shrink-0 items-center gap-2 transition-opacity duration-150 group-focus-within/row:opacity-0 group-hover/row:opacity-0 motion-reduce:transition-none">
          {isNew && <span data-slot="now-new-tag" className="shrink-0 rounded-sm bg-foreground/5 px-1.5 py-px text-[11px] text-muted-foreground">{t('now:newTag')}</span>}
          {unread && <span data-slot="unread-dot" aria-hidden className="size-1.5 shrink-0 rounded-full bg-primary" />}
          <span className="shrink-0 text-xs text-muted-foreground">{relativeTime(email.receivedAt, language, now)}</span>
        </span>
      </button>
      <div
        data-slot="now-row-actions"
        className="pointer-events-none absolute right-2 flex items-center gap-0.5 opacity-0 transition-opacity duration-150 group-focus-within/row:pointer-events-auto group-focus-within/row:opacity-100 group-hover/row:pointer-events-auto group-hover/row:opacity-100 motion-reduce:transition-none"
      >
        <IconButton
          label={email.isStarred ? t('mail:unstar') : t('mail:star')}
          size="icon-xs"
          disabled={quickActions.busy}
          aria-pressed={email.isStarred}
          className={cn('text-muted-foreground hover:text-foreground', email.isStarred && 'text-amber-500 hover:text-amber-500')}
          onClick={(event) => { event.stopPropagation(); quickActions.onToggleStar() }}
        >
          <HugeiconsIcon icon={StarIcon} fill={email.isStarred ? 'currentColor' : 'none'} className="size-3.5" />
        </IconButton>
        <IconButton
          label={email.isRead ? t('mail:markUnread') : t('mail:markRead')}
          size="icon-xs"
          disabled={quickActions.busy}
          className="text-muted-foreground hover:text-foreground"
          onClick={(event) => { event.stopPropagation(); quickActions.onMarkRead() }}
        >
          <HugeiconsIcon icon={MailOpen01Icon} className="size-3.5" />
        </IconButton>
        {quickActions.onArchive && (
          <IconButton
            label={t('mail:archive')}
            size="icon-xs"
            disabled={quickActions.busy}
            className="text-muted-foreground hover:text-foreground"
            onClick={(event) => { event.stopPropagation(); quickActions.onArchive?.() }}
          >
            <HugeiconsIcon icon={Archive02Icon} className="size-3.5" />
          </IconButton>
        )}
      </div>
    </div>
  )
}
