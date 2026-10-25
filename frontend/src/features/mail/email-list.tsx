import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Attachment01Icon, InboxIcon, Search01Icon, StarIcon } from '@hugeicons/core-free-icons'
import type { Account, EmailSummary, PreviewLines } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'
import { cn } from '@/lib/utils'
import { displayName, formatListTime } from './format'
import { SenderAvatar } from './sender-avatar'

export type ListFilter = 'all' | 'unread'

export type ListSyncStatus = {
  phase: 'waiting' | 'syncing' | 'idle' | 'failed'
  failure: string | null
  windowCapped: boolean
}

type EmailListProps = {
  title: string
  emails: EmailSummary[]
  /** When the list spans several accounts, each row names its owning mailbox (by name, not address) after the sender. */
  accounts: Account[] | null
  loading: boolean
  syncStatus: ListSyncStatus
  hasMore: boolean
  loadingMore: boolean
  refreshing: boolean
  /** Rows are the previous mailbox's while the new one loads. */
  stale: boolean
  loadMoreError: string | null
  error: string | null
  filter: ListFilter
  search: string
  selectedId: string | null
  /** Lines of body preview per row, from the appearance settings. */
  previewLines: PreviewLines
  onFilterChange: (filter: ListFilter) => void
  onSelect: (email: EmailSummary) => void
  onLoadMore: () => void
}

export function EmailList(props: EmailListProps) {
  useTranslation()
  const { title, filter, onFilterChange } = props
  return (
    <Tabs
      value={filter}
      onValueChange={(value) => onFilterChange(value as ListFilter)}
      className="flex h-full min-h-0 flex-col gap-0"
    >
      <div className="flex h-12 shrink-0 items-center gap-2 px-3">
        <h1 className="truncate text-base font-semibold">{title}</h1>
        <TabsList className="ml-auto">
          <TabsTrigger value="all">{t('mail:all')}</TabsTrigger>
          <TabsTrigger value="unread">{t('mail:unread')}</TabsTrigger>
        </TabsList>
      </div>
      <EmailRows {...props} />
    </Tabs>
  )
}

function EmailRows({ emails, accounts, loading, syncStatus, error, filter, search, selectedId, previewLines, onSelect, hasMore, loadingMore, refreshing, stale, loadMoreError, onLoadMore }: EmailListProps) {
  useTranslation()
  const pending = loading && emails.length === 0
  const showSkeleton = useDelayedFlag(pending)
  if (pending || showSkeleton) {
    // Nothing for the first moments: local data usually arrives before a
    // placeholder would be worth showing, and a flash is worse than a blink of blank.
    return showSkeleton ? (
      <div className="flex animate-in flex-col gap-2 px-3 pt-3 duration-150 fade-in-0" role="status" aria-label={t('mail:loadingMail')}>
        {Array.from({ length: 6 }, (_, index) => (
          <Skeleton key={index} className="h-20 w-full" />
        ))}
      </div>
    ) : <div className="min-h-0 flex-1" />
  }
  if (error) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>{t('mail:loadMailFailed')}</EmptyTitle>
          <EmptyDescription>{error}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }
  const syncTitle = {
    waiting: t('mail:waitingForSync'),
    syncing: t('mail:syncingMail'),
    failed: t('mail:mailSyncFailed'),
    idle: t('mail:noMailYet'),
  }[syncStatus.phase]
  const syncDescription = syncStatus.phase === 'failed'
    ? syncStatus.failure ?? t('mail:syncWillRetry')
    : syncStatus.phase === 'waiting'
      ? t('mail:folderSyncQueued')
      : t('mail:headersWillAppear')
  const filtered = Boolean(search) || filter === 'unread'
  if (emails.length === 0 && !filtered) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={InboxIcon} />
          </EmptyMedia>
          <EmptyTitle>{syncTitle}</EmptyTitle>
          <EmptyDescription>{syncDescription}</EmptyDescription>
          {syncStatus.windowCapped && <EmptyDescription>{t('mail:syncCoverageLimited')}</EmptyDescription>}
        </EmptyHeader>
      </Empty>
    )
  }
  if (emails.length === 0) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={search ? Search01Icon : InboxIcon} />
          </EmptyMedia>
          <EmptyTitle>{search ? t('mail:noMatchingMail') : filter === 'unread' ? t('mail:noUnreadMail') : t('mail:noMailYet')}</EmptyTitle>
          <EmptyDescription>{search ? t('mail:tryAnotherSearch') : t('mail:newMailWillAppear')}</EmptyDescription>
          {syncStatus.phase !== 'idle' && <EmptyDescription>{syncTitle} {syncDescription}</EmptyDescription>}
          {syncStatus.windowCapped && <EmptyDescription>{t('mail:syncCoverageLimited')}</EmptyDescription>}
        </EmptyHeader>
      </Empty>
    )
  }
  return (
    <ScrollArea className="min-h-0 flex-1 [&_[data-slot=scroll-area-viewport]]:!overflow-x-hidden [&_[data-slot=scroll-area-viewport]>div]:!block">
      <div
        role="list"
        aria-label={t('mail:messageList')}
        aria-busy={stale || undefined}
        className={cn(
          'flex animate-in flex-col gap-2 px-3 py-3 duration-150 fade-in-0 transition-opacity',
          stale && 'pointer-events-none opacity-50',
        )}
      >
        {emails.map((email) => (
          <EmailRow
            key={email.id}
            email={email}
            account={accounts?.find((account) => account.id === email.accountId) ?? null}
            selected={email.id === selectedId}
            previewLines={previewLines}
            onSelect={() => onSelect(email)}
          />
        ))}
      </div>
      {hasMore && (
        <div className="flex flex-col items-center gap-2 px-3 pb-3">
          {loadMoreError && <p role="alert" className="text-sm text-destructive">{loadMoreError}</p>}
          <Button variant="outline" disabled={loadingMore || refreshing} onClick={onLoadMore}>
            {loadingMore ? t('mail:loadingEllipsis') : t('mail:loadMore')}
          </Button>
        </div>
      )}
      {syncStatus.windowCapped && (
        <p className="px-3 pb-3 text-center text-xs text-muted-foreground">{t('mail:syncCoverageLimited')}</p>
      )}
    </ScrollArea>
  )
}

const PREVIEW_CLAMP: Record<Exclude<PreviewLines, 'none'>, string> = {
  one: 'line-clamp-1',
  two: 'line-clamp-2',
  three: 'line-clamp-3',
}

type EmailRowProps = { email: EmailSummary; account: Account | null; selected: boolean; previewLines: PreviewLines; onSelect: () => void }

function EmailRow({ email, account, selected, previewLines, onSelect }: EmailRowProps) {
  useTranslation()
  return (
    <button
      type="button"
      role="listitem"
      aria-current={selected ? 'true' : undefined}
      onClick={onSelect}
      className={cn(
        'flex w-full items-center gap-3 rounded-lg py-3 pr-3 pl-1.5 text-left text-sm transition-colors hover:bg-foreground/5',
        selected && 'bg-background shadow-xs hover:bg-background',
      )}
    >
      {/* Avatar and unread marker are centered on the row; the marker has a
          column of its own at the far left, so every dot sits on the same line
          however long the sender is. A fixed blue, not the accent color: a red
          or amber accent would read as an error or clash with the star. */}
      <span className="-mr-1.5 flex w-2 shrink-0 justify-center">
        {!email.isRead && <span aria-label={t('mail:unread')} className="size-2 rounded-full bg-blue-500 dark:bg-blue-400" />}
      </span>
      <SenderAvatar sender={email.from} accountId={email.accountId} />
      <div className="flex min-w-0 flex-1 flex-col items-start gap-2">
        <div className="flex w-full flex-col gap-1">
          <div className="flex items-center gap-2">
            {/* Sender first; in a view that spans accounts, the owning mailbox follows it,
                muted, and gives way to the sender before the time ever does. */}
            <span className="flex min-w-0 items-baseline gap-1.5">
              <span className={cn('truncate', email.isRead ? 'font-normal' : 'font-semibold')}>{displayName(email.from)}</span>
              {account && (
                <>
                  <span aria-hidden className="shrink-0 text-xs text-muted-foreground">|</span>
                  <span className="min-w-0 truncate text-xs text-muted-foreground" title={account.emailAddress}>{account.displayName}</span>
                </>
              )}
            </span>
            <span className={cn('ml-auto shrink-0 text-xs', selected ? 'text-foreground' : 'text-muted-foreground')}>
              {formatListTime(email.receivedAt)}
            </span>
          </div>
          {/* Unread is told by weight too, never by the dot's color alone. */}
          <div className={cn('flex items-center gap-1.5 text-xs', email.isRead ? 'font-normal text-foreground/75' : 'font-semibold')}>
            {email.isStarred && (
              <HugeiconsIcon icon={StarIcon} aria-label={t('mail:starred')} fill="currentColor" className="size-3.5 shrink-0 text-amber-500" />
            )}
            <span className="truncate">{email.subject || t('mail:noSubject')}</span>
            {email.hasAttachment && (
              <HugeiconsIcon icon={Attachment01Icon} aria-label={t('mail:hasAttachment')} className="size-3.5 shrink-0 text-muted-foreground" />
            )}
          </div>
        </div>
        {email.preview && previewLines !== 'none' && (
          <p className={cn(PREVIEW_CLAMP[previewLines], 'text-xs text-muted-foreground')}>{email.preview}</p>
        )}
      </div>
    </button>
  )
}
