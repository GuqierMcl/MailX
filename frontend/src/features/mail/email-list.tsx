import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { defaultRangeExtractor, useVirtualizer, type Range } from '@tanstack/react-virtual'
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
import { isGivenUp, useGaveUpVersion } from './preview-gave-up'
import { SenderAvatar } from './sender-avatar'
import { usePreviewRequests } from './use-preview-requests'

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
  /** Asks for the previews of the listed rows; the complete wanted set every time. */
  onRequestPreviews?: (emailIds: string[]) => void
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

function EmailRows({ emails, accounts, loading, syncStatus, error, filter, search, selectedId, previewLines, onRequestPreviews, onSelect, hasMore, loadingMore, refreshing, stale, loadMoreError, onLoadMore }: EmailListProps) {
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
    <VirtualRows
      emails={emails} accounts={accounts} stale={stale} selectedId={selectedId} previewLines={previewLines} onRequestPreviews={onRequestPreviews}
      hasMore={hasMore} loadingMore={loadingMore} refreshing={refreshing} loadMoreError={loadMoreError}
      windowCapped={syncStatus.windowCapped} onSelect={onSelect} onLoadMore={onLoadMore}
    />
  )
}

// Every row has the same height for a given preview setting (sender and subject are
// single-line, the preview slot is reserved), so the virtualizer can be told exactly.
// Row: py-3 (1.5rem) + sender line text-sm (1.25rem) + gap-1 (0.25rem) + subject
// line text-xs (1rem) = 4rem; with a preview, gap-2 (0.5rem) + one 1rem per line.
const PREVIEW_ROWS: Record<PreviewLines, number> = { none: 0, one: 1, two: 2, three: 3 }
const ROW_GAP = 8
const LIST_PADDING = 12
const OVERSCAN = 8
/** Ask for the next page once the last visible row is this close to the end. */
const LOAD_MORE_THRESHOLD = 10

function rowHeight(previewLines: PreviewLines) {
  const rootFontSize = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16
  const rem = 4 + (previewLines === 'none' ? 0 : 0.5 + PREVIEW_ROWS[previewLines])
  return rem * rootFontSize
}

type VirtualRowsProps = {
  emails: EmailSummary[]
  accounts: Account[] | null
  stale: boolean
  selectedId: string | null
  previewLines: PreviewLines
  onRequestPreviews?: (emailIds: string[]) => void
  hasMore: boolean
  loadingMore: boolean
  refreshing: boolean
  loadMoreError: string | null
  windowCapped: boolean
  onSelect: (email: EmailSummary) => void
  onLoadMore: () => void
}

function VirtualRows({ emails, accounts, stale, selectedId, previewLines, onRequestPreviews, hasMore, loadingMore, refreshing, loadMoreError, windowCapped, onSelect, onLoadMore }: VirtualRowsProps) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const [focusedId, setFocusedId] = useState<string | null>(null)
  // The selected and the focused row stay mounted however far they scroll away,
  // so the open message keeps its highlight and keyboard focus is never dropped.
  const pinned = useMemo(() => {
    const indexes: number[] = []
    for (const id of [selectedId, focusedId]) {
      const index = id === null ? -1 : emails.findIndex((email) => email.id === id)
      if (index >= 0) indexes.push(index)
    }
    return indexes
  }, [emails, selectedId, focusedId])
  const rangeExtractor = useCallback(
    (range: Range) => [...new Set([...defaultRangeExtractor(range), ...pinned])].sort((a, b) => a - b),
    [pinned],
  )
  // The React Compiler cannot memoize the virtualizer, which is fine: nothing here relies on it.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: emails.length,
    getScrollElement: () => viewportRef.current,
    getItemKey: (index) => emails[index].id,
    estimateSize: () => rowHeight(previewLines),
    // A row that is not laid out (zero height) keeps the estimate instead of collapsing.
    measureElement: (element) => element.getBoundingClientRect().height || rowHeight(previewLines),
    gap: ROW_GAP,
    paddingStart: LIST_PADDING,
    paddingEnd: LIST_PADDING,
    overscan: OVERSCAN,
    rangeExtractor,
  })

  // Row heights follow the preview setting and the root font size (rem), so a change
  // to either drops the cached sizes.
  useEffect(() => {
    virtualizer.measure()
  }, [virtualizer, previewLines])
  useEffect(() => {
    let fontSize = getComputedStyle(document.documentElement).fontSize
    const observer = new MutationObserver(() => {
      const next = getComputedStyle(document.documentElement).fontSize
      if (next !== fontSize) {
        fontSize = next
        virtualizer.measure()
      }
    })
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['style'] })
    return () => observer.disconnect()
  }, [virtualizer])

  // Load the next page before the user reaches the end. A failed load is retried
  // only by the button, never automatically.
  const lastVisible = virtualizer.range?.endIndex ?? -1
  useEffect(() => {
    if (hasMore && !loadingMore && !refreshing && !loadMoreError && lastVisible >= emails.length - LOAD_MORE_THRESHOLD) onLoadMore()
  }, [hasMore, loadingMore, refreshing, loadMoreError, lastVisible, emails.length, onLoadMore])

  // Previews are fetched for the rows that are rendered (visible plus overscan).
  const virtualItems = virtualizer.getVirtualItems()
  const rendered = virtualItems.map((item) => emails[item.index])
  usePreviewRequests(onRequestPreviews, rendered, previewLines)
  useGaveUpVersion()

  return (
    <ScrollArea
      viewportRef={viewportRef}
      className="min-h-0 flex-1 [&_[data-slot=scroll-area-viewport]]:!overflow-x-hidden [&_[data-slot=scroll-area-viewport]>div]:!block"
    >
      <div
        role="list"
        aria-label={t('mail:messageList')}
        aria-busy={stale || undefined}
        style={{ height: virtualizer.getTotalSize() }}
        className={cn(
          'relative animate-in px-3 duration-150 fade-in-0 transition-opacity',
          stale && 'pointer-events-none opacity-50',
        )}
        onFocus={(event) => {
          const row = (event.target as HTMLElement).closest<HTMLElement>('[data-index]')
          setFocusedId(row ? emails[Number(row.dataset.index)]?.id ?? null : null)
        }}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocusedId(null)
        }}
      >
        {virtualItems.map((item) => {
          const email = emails[item.index]
          return (
            <EmailRow
              key={item.key}
              ref={virtualizer.measureElement}
              index={item.index}
              count={emails.length}
              style={{ transform: `translateY(${item.start}px)` }}
              email={email}
              account={accounts?.find((account) => account.id === email.accountId) ?? null}
              selected={email.id === selectedId}
              previewLines={previewLines}
              onSelect={() => onSelect(email)}
            />
          )
        })}
      </div>
      {hasMore && (
        <div className="flex flex-col items-center gap-2 px-3 pb-3">
          {loadMoreError && <p role="alert" className="text-sm text-destructive">{loadMoreError}</p>}
          <Button variant="outline" disabled={loadingMore || refreshing} onClick={onLoadMore}>
            {loadingMore ? t('mail:loadingEllipsis') : t('mail:loadMore')}
          </Button>
        </div>
      )}
      {windowCapped && (
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

const PREVIEW_HEIGHT: Record<Exclude<PreviewLines, 'none'>, string> = {
  one: 'h-4',
  two: 'h-8',
  three: 'h-12',
}

/**
 * What the preview slot shows. A row still waiting for its preview says so on the
 * first line; failed rows and rows given up on stay blank. A downloaded body is
 * always a ready preview.
 */
function previewText(email: EmailSummary) {
  if (email.bodyState === 'ready' || email.previewState === 'ready') return email.preview
  if (email.previewState === 'missing' && !isGivenUp(email.id)) return t('mail:previewLoading')
  return ''
}

type EmailRowProps = {
  email: EmailSummary
  account: Account | null
  selected: boolean
  previewLines: PreviewLines
  index: number
  count: number
  style: React.CSSProperties
  ref: React.Ref<HTMLButtonElement>
  onSelect: () => void
}

function EmailRow({ email, account, selected, previewLines, index, count, style, ref, onSelect }: EmailRowProps) {
  useTranslation()
  return (
    <button
      ref={ref}
      type="button"
      role="listitem"
      aria-setsize={count}
      aria-posinset={index + 1}
      data-index={index}
      style={style}
      aria-current={selected ? 'true' : undefined}
      onClick={onSelect}
      className={cn(
        'absolute inset-x-3 flex items-center gap-3 rounded-lg py-3 pr-3 pl-1.5 text-left text-sm transition-colors hover:bg-foreground/5',
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
        {/* The slot is always reserved, even without a preview, so every row has one height. */}
        {previewLines !== 'none' && (
          <p className={cn(PREVIEW_CLAMP[previewLines], PREVIEW_HEIGHT[previewLines], 'w-full overflow-hidden text-xs text-muted-foreground')}>{previewText(email)}</p>
        )}
      </div>
    </button>
  )
}
