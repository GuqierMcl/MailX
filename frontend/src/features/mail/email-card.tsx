import { t } from '@/i18n'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Archive02Icon, ArchiveRestoreIcon, Attachment01Icon, Delete02Icon, StarIcon } from '@hugeicons/core-free-icons'
import type { Account, EmailSummary, PreviewLines, SnippetSegment } from '@/generated/accounts'
import { IconButton } from '@/components/icon-button'
import { cn } from '@/lib/utils'
import { displayName, formatListTime, recipientsLabel } from './format'
import { LocalOnlyMarker } from './local-only-marker'
import { isGivenUp } from './preview-gave-up'
import { SenderAvatar } from './sender-avatar'
import { ReadStatusButton } from './read-status-button'

const PREVIEW_CLAMP: Record<Exclude<PreviewLines, 'none'>, string> = {
  one: 'line-clamp-1',
  two: 'line-clamp-2',
  three: 'line-clamp-3',
}

const PREVIEW_HEIGHT: Record<Exclude<PreviewLines, 'none'>, string> = {
  one: 'h-4',
  two: 'h-8',
  three: 'h-12',
}

const ACTION_FADE_MS = 150
const QUICK_ACTION_CLASS = 'bg-transparent transition-colors hover:bg-transparent dark:hover:bg-transparent active:not-aria-[haspopup]:translate-y-0'

type EmailCardProps = {
  email: EmailSummary
  sentView: boolean
  account: Account | null
  selected: boolean
  previewLines: PreviewLines
  index: number
  count: number
  style: React.CSSProperties
  ref: React.Ref<HTMLDivElement>
  onFlag?: (email: EmailSummary, flag: 'read' | 'starred', wanted: boolean) => void
  onArchive?: (email: EmailSummary, archive: boolean) => void
  onDelete?: (email: EmailSummary) => void
  onRemoveLocal?: (email: EmailSummary) => void
  canArchive?: (email: EmailSummary) => boolean
  onSelect: () => void
}

/** A message row with its state slot and quick actions kept outside the main link. */
export function EmailCard({ email, sentView, account, selected, previewLines, index, count, style, ref, onFlag, onArchive, onDelete, onRemoveLocal, canArchive, onSelect }: EmailCardProps) {
  useTranslation()
  // In a sent view the row is about who the mail went to. The avatar follows
  // the same person, so it never shows the account owner on every row.
  const counterpart = sentView ? email.to.addresses[0] ?? email.from : email.from
  const canChangeFlags = Boolean(onFlag) && !email.localOnly
  const canChangeArchive = Boolean(onArchive && !email.localOnly && canArchive?.(email))
  const canDelete = Boolean(onDelete) && !email.localOnly
  const canRemoveLocal = Boolean(onRemoveLocal) && email.localOnly
  const hasActions = canChangeFlags || canChangeArchive || canDelete || canRemoveLocal
  // The shortest rows cannot fit three comfortable targets beside the avatar.
  const verticalActions = previewLines !== 'none'
  const hasCornerActions = canDelete || canRemoveLocal || (!verticalActions && (canChangeFlags || canChangeArchive))
  const [actionsMounted, setActionsMounted] = useState(false)
  const [actionsVisible, setActionsVisible] = useState(false)
  const wantsActionsVisible = useRef(false)
  const pointerDown = useRef(false)
  const keyboardFocus = useRef(false)
  const hideTimer = useRef<number | null>(null)
  const clearHideTimer = () => {
    if (hideTimer.current !== null) {
      window.clearTimeout(hideTimer.current)
      hideTimer.current = null
    }
  }
  const showActions = () => {
    if (!hasActions) return
    wantsActionsVisible.current = true
    clearHideTimer()
    if (actionsMounted) setActionsVisible(true)
    else {
      setActionsVisible(false)
      setActionsMounted(true)
    }
  }
  const hideActions = () => {
    if (!hasActions) return
    wantsActionsVisible.current = false
    setActionsVisible(false)
    clearHideTimer()
    hideTimer.current = window.setTimeout(() => {
      setActionsMounted(false)
      hideTimer.current = null
    }, ACTION_FADE_MS)
  }
  useEffect(() => {
    if (!actionsMounted) return
    // Paint the transparent controls once before fading them in.
    let revealFrame = window.requestAnimationFrame(() => {
      revealFrame = window.requestAnimationFrame(() => {
        if (wantsActionsVisible.current) setActionsVisible(true)
      })
    })
    return () => window.cancelAnimationFrame(revealFrame)
  }, [actionsMounted])
  useEffect(() => () => {
    if (hideTimer.current !== null) window.clearTimeout(hideTimer.current)
  }, [])
  const actionVisibility = cn(
    'transition-opacity duration-150 ease-out motion-reduce:transition-none',
    actionsVisible ? 'opacity-100' : 'opacity-0',
  )
  const starButton = (
    <IconButton
      data-row-action
      label={email.isStarred ? t('mail:unstar') : t('mail:star')}
      aria-pressed={email.isStarred}
      className={cn(
        QUICK_ACTION_CLASS,
        verticalActions && 'size-6 min-h-[24px] min-w-[24px]',
        email.isStarred && 'text-amber-500 hover:text-amber-500',
      )}
      onClick={() => onFlag?.(email, 'starred', !email.isStarred)}
    >
      <HugeiconsIcon icon={StarIcon} aria-label={verticalActions && email.isStarred ? t('mail:starred') : undefined} fill={email.isStarred ? 'currentColor' : 'none'} />
    </IconButton>
  )
  const archiveButton = (
    <IconButton
      data-row-action
      label={email.archived ? t('mail:unarchive') : t('mail:archive')}
      className={cn(QUICK_ACTION_CLASS, verticalActions && 'size-6 min-h-[24px] min-w-[24px]')}
      onClick={() => onArchive?.(email, !email.archived)}
    >
      <HugeiconsIcon icon={email.archived ? ArchiveRestoreIcon : Archive02Icon} />
    </IconButton>
  )
  const starMarker = email.isStarred && (
    <HugeiconsIcon icon={StarIcon} aria-label={t('mail:starred')} fill="currentColor" className="size-3.5 shrink-0 text-amber-500" />
  )
  const hideTime = hasCornerActions && actionsVisible
  return (
    <div
      ref={ref}
      role="listitem"
      aria-setsize={count}
      aria-posinset={index + 1}
      aria-current={selected ? 'true' : undefined}
      data-index={index}
      style={style}
      onMouseEnter={showActions}
      onMouseLeave={(event) => {
        pointerDown.current = false
        if (!keyboardFocus.current || !event.currentTarget.contains(document.activeElement)) hideActions()
      }}
      onPointerDownCapture={() => {
        pointerDown.current = true
        keyboardFocus.current = false
      }}
      onPointerUpCapture={() => { pointerDown.current = false }}
      onPointerCancelCapture={() => { pointerDown.current = false }}
      onFocusCapture={() => {
        // A pointer click leaves DOM focus behind, but only keyboard focus
        // should keep quick actions open once the pointer leaves the row.
        keyboardFocus.current = !pointerDown.current
        showActions()
      }}
      onKeyDownCapture={() => {
        keyboardFocus.current = true
        showActions()
      }}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          keyboardFocus.current = false
          pointerDown.current = false
          hideActions()
        }
      }}
      onClick={(event) => {
        if ((event.target as HTMLElement).closest('[data-row-action]')) return
        onSelect()
      }}
      className={cn(
        'group/row absolute inset-x-3 flex cursor-pointer items-center gap-1 rounded-lg py-3 pr-3 pl-1.5 text-left text-sm transition-colors hover:bg-foreground/5 focus-within:bg-foreground/5',
        selected && 'mail-selected',
      )}
    >
      <button
        type="button"
        className="order-2 flex min-w-0 flex-1 items-center gap-3 text-left outline-none"
      >
        <SenderAvatar sender={counterpart} accountId={email.accountId} />
        <div className="flex min-w-0 flex-1 flex-col items-start gap-2">
          <div className="flex w-full flex-col gap-1">
            <div className={cn('flex items-center gap-2', !verticalActions && hasActions ? 'pr-24' : 'pr-16')}>
              {/* Sender first; in a view that spans accounts, the owning mailbox follows it,
                  muted, and gives way to the sender before the time ever does. */}
              <span className="flex min-w-0 items-baseline gap-1.5">
                <span className="truncate font-normal">{sentView ? recipientsLabel(email.to) : displayName(email.from)}</span>
                {account && (
                  <>
                    <span aria-hidden className="shrink-0 text-xs text-muted-foreground">|</span>
                    <span className="min-w-0 truncate text-xs text-muted-foreground" title={account.emailAddress}>{account.displayName}</span>
                  </>
                )}
              </span>
              {email.localOnly && <LocalOnlyMarker className="ml-auto" />}
              {email.archived && <span className="ml-auto shrink-0 text-xs text-muted-foreground">{t('mail:archivedMarker')}</span>}
            </div>
            {/* The blue dot is the only visual read-state marker in the row. */}
            <div className="flex items-center gap-1.5 text-xs text-foreground/75">
              {!verticalActions && starMarker}
              <span className="truncate">{email.subject || t('mail:noSubject')}</span>
              {email.hasAttachment && (
                <HugeiconsIcon icon={Attachment01Icon} aria-label={t('mail:hasAttachment')} className="size-3.5 shrink-0 text-muted-foreground" />
              )}
            </div>
          </div>
          {/* The slot is always reserved, even without a preview, so every row has one height. */}
          {previewLines !== 'none' && (
            <p className={cn(PREVIEW_CLAMP[previewLines], PREVIEW_HEIGHT[previewLines], 'w-full overflow-hidden text-xs text-muted-foreground')}>
              {email.matchSnippet ? <SnippetText segments={email.matchSnippet} /> : previewText(email)}
            </p>
          )}
        </div>
      </button>
      {/* This slot stays centred beside the avatar. Its absolute actions use the
          space above and below without changing row height or text width.
          DOM order keeps the main message first, then star, read and archive. */}
      <div data-slot="email-state-actions" className="relative order-1 flex size-7 shrink-0 items-center justify-center">
        {verticalActions && (
          <div className="absolute inset-x-0 -top-6.5 flex h-6 items-center justify-center">
            {canChangeFlags && (actionsMounted || email.isStarred) ? (
              <span className={email.isStarred ? undefined : actionVisibility}>{starButton}</span>
            ) : starMarker}
          </div>
        )}
        <ReadStatusButton
          data-row-action
          isRead={email.isRead}
          interactive={canChangeFlags && actionsMounted}
          ringClassName={actionVisibility}
          onClick={() => onFlag?.(email, 'read', !email.isRead)}
        />
        {verticalActions && canChangeArchive && actionsMounted && (
          <div className={cn('absolute inset-x-0 top-7.5 flex h-6 items-center justify-center', actionVisibility)}>
            {archiveButton}
          </div>
        )}
      </div>
      <div className="order-3 absolute top-2 right-3 flex h-7 items-center justify-end">
        <time
          dateTime={email.receivedAt}
          aria-hidden={hideTime || undefined}
          className={cn(
            'shrink-0 text-xs transition-opacity duration-150 ease-out motion-reduce:transition-none',
            hideTime ? 'opacity-0' : 'opacity-100',
            selected ? 'text-foreground' : 'text-muted-foreground',
          )}
        >
          {formatListTime(email.receivedAt)}
        </time>
        {hasCornerActions && actionsMounted && (
          <div className={cn('absolute right-0 flex items-center gap-0.5', actionVisibility)}>
            {!verticalActions && canChangeFlags && starButton}
            {!verticalActions && canChangeArchive && archiveButton}
            {canDelete && (
              <IconButton
                data-row-action
                label={t('mail:delete')}
                className={cn(QUICK_ACTION_CLASS, 'text-destructive hover:text-destructive')}
                onClick={() => onDelete?.(email)}
              >
                <HugeiconsIcon icon={Delete02Icon} />
              </IconButton>
            )}
            {canRemoveLocal && (
              <IconButton
                data-row-action
                label={t('mail:removeLocalCopy')}
                className={cn(QUICK_ACTION_CLASS, 'text-destructive hover:text-destructive')}
                onClick={() => onRemoveLocal?.(email)}
              >
                <HugeiconsIcon icon={Delete02Icon} />
              </IconButton>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

/** What the preview slot shows while the body is fetched or unavailable. */
function previewText(email: EmailSummary) {
  if (email.bodyState === 'ready' || email.previewState === 'ready') return email.preview
  if (email.previewState === 'missing' && !isGivenUp(email.id)) return t('mail:previewLoading')
  return ''
}

/** Search hits use a faint wash and stronger text, while other segments remain plain text. */
function SnippetText({ segments }: { segments: SnippetSegment[] }) {
  return segments.map((segment, index) => segment.hit
    ? <mark key={index} className="rounded-xs bg-foreground/10 text-foreground">{segment.text}</mark>
    : <span key={index}>{segment.text}</span>)
}
