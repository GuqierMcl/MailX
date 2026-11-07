import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { defaultRangeExtractor, useVirtualizer, type Range } from '@tanstack/react-virtual'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { InboxIcon, Search01Icon } from '@hugeicons/core-free-icons'
import type { Account, EmailSummary, PreviewLines } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'
import { cn } from '@/lib/utils'
import { EmailCard } from './email-card'
import { useGaveUpVersion } from './preview-gave-up'
import { usePreviewRequests } from './use-preview-requests'

export type ListFilter = 'all' | 'unread'

export type ListSyncStatus = {
  phase: 'waiting' | 'syncing' | 'idle' | 'failed'
  failure: string | null
  windowCapped: boolean
}

type EmailListProps = {
  title: string
  /** A sent-role view (a Sent folder or the unified Sent): rows name the recipients, not the sender. */
  sentView?: boolean
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
  /** Messages in scope whose body is in the local search index; shown as a hint while searching. */
  indexedBodies?: number
  selectedId: string | null
  /** Lines of body preview per row, from the appearance settings. */
  previewLines: PreviewLines
  /** Asks for the previews of the listed rows; the complete wanted set every time. */
  onRequestPreviews?: (emailIds: string[]) => void
  /** Changes a message read or starred flag from the row action rail. */
  onFlag?: (email: EmailSummary, flag: 'read' | 'starred', wanted: boolean) => void
  /** Archives or restores a message when the current row can be changed locally. */
  onArchive?: (email: EmailSummary, archive: boolean) => void
  /** Deletes a server message through the existing undo / confirmation flow. */
  onDelete?: (email: EmailSummary) => void
  /** Requests confirmation before removing a local-only sent copy. */
  onRemoveLocal?: (email: EmailSummary) => void
  /** Whether local archive is available for this message's current folder. */
  canArchive?: (email: EmailSummary) => boolean
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

function EmailRows({ sentView = false, emails, accounts, loading, syncStatus, error, filter, search, indexedBodies = 0, selectedId, previewLines, onRequestPreviews, onFlag, onArchive, onDelete, onRemoveLocal, canArchive, onSelect, hasMore, loadingMore, refreshing, stale, loadMoreError, onLoadMore }: EmailListProps) {
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
      sentView={sentView} emails={emails} accounts={accounts} stale={stale} selectedId={selectedId} previewLines={previewLines} onRequestPreviews={onRequestPreviews}
      onFlag={onFlag} onArchive={onArchive} onDelete={onDelete} onRemoveLocal={onRemoveLocal} canArchive={canArchive}
      hasMore={hasMore} loadingMore={loadingMore} refreshing={refreshing} loadMoreError={loadMoreError}
      windowCapped={syncStatus.windowCapped} searchHint={search ? t('mail:searchedBodies', { count: indexedBodies }) : null} onSelect={onSelect} onLoadMore={onLoadMore}
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
  sentView: boolean
  emails: EmailSummary[]
  accounts: Account[] | null
  stale: boolean
  selectedId: string | null
  previewLines: PreviewLines
  onRequestPreviews?: (emailIds: string[]) => void
  onFlag?: (email: EmailSummary, flag: 'read' | 'starred', wanted: boolean) => void
  onArchive?: (email: EmailSummary, archive: boolean) => void
  onDelete?: (email: EmailSummary) => void
  onRemoveLocal?: (email: EmailSummary) => void
  canArchive?: (email: EmailSummary) => boolean
  hasMore: boolean
  loadingMore: boolean
  refreshing: boolean
  loadMoreError: string | null
  windowCapped: boolean
  /** What a search covered, muted at the top of the results; null when not searching. */
  searchHint: string | null
  onSelect: (email: EmailSummary) => void
  onLoadMore: () => void
}

function VirtualRows({ sentView, emails, accounts, stale, selectedId, previewLines, onRequestPreviews, onFlag, onArchive, onDelete, onRemoveLocal, canArchive, hasMore, loadingMore, refreshing, loadMoreError, windowCapped, searchHint, onSelect, onLoadMore }: VirtualRowsProps) {
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
      {searchHint && (
        <p className="px-4 pt-3 text-xs text-muted-foreground">
          {windowCapped ? `${searchHint} ${t('mail:syncCoverageLimited')}` : searchHint}
        </p>
      )}
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
            <EmailCard
              key={item.key}
              ref={virtualizer.measureElement}
              index={item.index}
              count={emails.length}
              style={{ transform: `translateY(${item.start}px)` }}
              email={email}
              sentView={sentView}
              account={accounts?.find((account) => account.id === email.accountId) ?? null}
              selected={email.id === selectedId}
              previewLines={previewLines}
              onFlag={onFlag}
              onArchive={onArchive}
              onDelete={onDelete}
              onRemoveLocal={onRemoveLocal}
              canArchive={canArchive}
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
      {windowCapped && !searchHint && (
        <p className="px-3 pb-3 text-center text-xs text-muted-foreground">{t('mail:syncCoverageLimited')}</p>
      )}
    </ScrollArea>
  )
}
