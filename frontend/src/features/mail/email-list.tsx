import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { ContextMenu as ContextMenuPrimitive } from '@base-ui/react/context-menu'
import type { BaseUIEvent } from '@base-ui/react/types'
import { defaultRangeExtractor, useVirtualizer, type Range } from '@tanstack/react-virtual'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowLeft01Icon, InboxIcon, Search01Icon } from '@hugeicons/core-free-icons'
import type { Account, EmailSummary, PreviewLines } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'
import { cn } from '@/lib/utils'
import { EmailCard, type RowClick } from './email-card'
import { EmailContextMenuContent } from './email-context-menu'
import type { EmailMenuCallbacks, RespondKind } from './email-menu-actions'
import { SelectionHeader, type SelectionActions } from './selection-header'
import { escapeBelongsToSomethingElse } from './use-close-on-escape'
import type { ListSelection } from './use-list-selection'
import { useGaveUpVersion } from './preview-gave-up'
import { usePreviewRequests } from './use-preview-requests'

export type ListFilter = 'all' | 'unread'

export type ListSyncStatus = {
  phase: 'waiting' | 'syncing' | 'idle' | 'failed'
  failure: string | null
  windowCapped: boolean
}

/** Shown in the header while the list holds one sender's mail (the contact view). */
export type ContactHeading = {
  name: string | null
  address: string
  /** The view this one returns to, e.g. "收件箱". */
  backTitle: string
  onBack: () => void
}

type EmailListProps = Pick<EmailMenuCallbacks, 'onViewContact' | 'onAggregate' | 'isAggregated'> & {
  title: string
  /** Set in the contact view: back button and the contact's name replace the plain title. */
  contact?: ContactHeading
  /** Where the list starts scrolling, in px; used to come back to a remembered position. */
  initialScrollTop?: number
  /** Reports the scroll offset as the user scrolls. */
  onScrollOffset?: (offset: number) => void
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
  /** Starts a reply, reply-all or forward from the right-click menu (opens the composer window). */
  onRespond?: (email: EmailSummary, kind: RespondKind) => void
  onFilterChange: (filter: ListFilter) => void
  onSelect: (email: EmailSummary) => void
  onLoadMore: () => void
  /** Multi-select; without it the list has no "Select" entry. */
  selection?: ListSelection
  /** What the selection header does with the picked messages. */
  selectionActions?: SelectionActions
}

export function EmailList(props: EmailListProps) {
  useTranslation()
  const { title, contact, filter, onFilterChange, selection, selectionActions } = props
  const selecting = Boolean(selection?.active && selectionActions)
  useSelectionKeys(selecting ? selection : undefined, selectionActions)
  return (
    <Tabs
      value={filter}
      onValueChange={(value) => onFilterChange(value as ListFilter)}
      className="flex h-full min-h-0 flex-col gap-0"
    >
      <div className="flex h-12 shrink-0 items-center gap-2 px-3">
        {selecting && selection && selectionActions ? (
          <>
            <h1 className="sr-only">{title}</h1>
            <SelectionHeader selection={selection} actions={selectionActions} />
          </>
        ) : (
          <>
            {contact ? <ContactTitle contact={contact} /> : <h1 className="truncate text-base font-semibold">{title}</h1>}
            {selection && selectionActions && (
              <Button variant="ghost" size="sm" className="ml-auto" onClick={selection.enter}>{t('mail:select')}</Button>
            )}
            <TabsList className={selection && selectionActions ? undefined : 'ml-auto'}>
              <TabsTrigger value="all">{t('mail:all')}</TabsTrigger>
              <TabsTrigger value="unread">{t('mail:unread')}</TabsTrigger>
            </TabsList>
          </>
        )}
      </div>
      {selecting && selection?.truncated && (
        <p className="-mt-1 shrink-0 px-4 pb-1 text-xs text-muted-foreground">{t('mail:selectionLimit')}</p>
      )}
      <EmailRows {...props} selection={selectionActions ? selection : undefined} />
    </Tabs>
  )
}

/** Back button, then the contact's name with the address muted under it. */
function ContactTitle({ contact }: { contact: ContactHeading }) {
  const name = contact.name?.trim() || null
  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        className="-ml-1 max-w-28 shrink-0 px-2"
        aria-label={t('mail:backTo', { title: contact.backTitle })}
        onClick={contact.onBack}
      >
        <HugeiconsIcon icon={ArrowLeft01Icon} />
        <span className="truncate">{contact.backTitle}</span>
      </Button>
      <div className="flex min-w-0 flex-col leading-tight">
        <h1 className="truncate text-base font-semibold" title={name ?? contact.address}>{name ?? contact.address}</h1>
        {name && <p className="truncate text-xs text-muted-foreground" title={contact.address}>{contact.address}</p>}
      </div>
    </>
  )
}

/** Esc leaves selection mode; Cmd/Ctrl+A selects everything in scope, outside text fields. */
function useSelectionKeys(selection: ListSelection | undefined, actions: SelectionActions | undefined) {
  const exit = selection?.exit
  const selectAll = actions?.onSelectAll
  useEffect(() => {
    if (!exit || !selectAll) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (escapeBelongsToSomethingElse(event)) return
      if (event.key === 'Escape') {
        event.preventDefault()
        exit()
      } else if (event.key.toLowerCase() === 'a' && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) {
        event.preventDefault()
        selectAll()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [exit, selectAll])
}

function EmailRows({ contact, initialScrollTop, onScrollOffset, onViewContact, onAggregate, isAggregated, sentView = false, emails, accounts, loading, syncStatus, error, filter, search, indexedBodies = 0, selectedId, previewLines, onRequestPreviews, onFlag, onArchive, onDelete, onRemoveLocal, canArchive, onRespond, onSelect, hasMore, loadingMore, refreshing, stale, loadMoreError, onLoadMore, selection }: EmailListProps) {
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
      onFlag={onFlag} onArchive={onArchive} onDelete={onDelete} onRemoveLocal={onRemoveLocal} canArchive={canArchive} onRespond={onRespond}
      hasMore={hasMore} loadingMore={loadingMore} refreshing={refreshing} loadMoreError={loadMoreError}
      windowCapped={syncStatus.windowCapped} searchHint={search ? t('mail:searchedBodies', { count: indexedBodies }) : null} onSelect={onSelect} onLoadMore={onLoadMore}
      selection={selection}
      viewingContact={contact?.address ?? null} onViewContact={onViewContact} onAggregate={onAggregate} isAggregated={isAggregated}
      initialScrollTop={initialScrollTop} onScrollOffset={onScrollOffset}
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
  onRespond?: (email: EmailSummary, kind: RespondKind) => void
  hasMore: boolean
  loadingMore: boolean
  refreshing: boolean
  loadMoreError: string | null
  windowCapped: boolean
  /** What a search covered, muted at the top of the results; null when not searching. */
  searchHint: string | null
  onSelect: (email: EmailSummary) => void
  onLoadMore: () => void
  /** Present while picking messages. */
  selection?: ListSelection
  viewingContact: string | null
  initialScrollTop?: number
  onScrollOffset?: (offset: number) => void
} & Pick<EmailMenuCallbacks, 'onViewContact' | 'onAggregate' | 'isAggregated'>

function VirtualRows({ sentView, emails, accounts, stale, selectedId, previewLines, onRequestPreviews, onFlag, onArchive, onDelete, onRemoveLocal, canArchive, onRespond, hasMore, loadingMore, refreshing, loadMoreError, windowCapped, searchHint, onSelect, onLoadMore, selection, viewingContact, onViewContact, onAggregate, isAggregated, initialScrollTop, onScrollOffset }: VirtualRowsProps) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const [focusedId, setFocusedId] = useState<string | null>(null)
  // One right-click menu serves the whole list. The snapshot outlives the open
  // state so the closing animation keeps its content.
  const [menu, setMenu] = useState<{ email: EmailSummary; open: boolean } | null>(null)
  // Where focus was when the menu opened; it goes back there when the menu closes.
  const menuReturnFocus = useRef<HTMLElement | null>(null)
  const menuOpenId = menu?.open ? menu.email.id : null
  // Old rows of the previous mailbox, and selection mode (whose header holds the
  // batch actions), offer no per-message menu.
  const menuDisabled = stale || Boolean(selection?.active)
  // The selected, the focused and the menu's row stay mounted however far they
  // scroll away, so the open message keeps its highlight, keyboard focus is never
  // dropped and the menu never loses its row.
  const pinned = useMemo(() => {
    const indexes: number[] = []
    for (const id of [selectedId, focusedId, menuOpenId]) {
      const index = id === null ? -1 : emails.findIndex((email) => email.id === id)
      if (index >= 0) indexes.push(index)
    }
    return indexes
  }, [emails, selectedId, focusedId, menuOpenId])
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
    // Coming back to a list: start where the user left it.
    initialOffset: initialScrollTop,
  })

  // Report the scroll offset so the caller can come back to it later.
  useEffect(() => {
    const viewport = viewportRef.current
    if (!viewport || !onScrollOffset) return
    const onScroll = () => onScrollOffset(viewport.scrollTop)
    viewport.addEventListener('scroll', onScroll, { passive: true })
    return () => viewport.removeEventListener('scroll', onScroll)
  }, [onScrollOffset])

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

  // Outside selection mode a plain click opens; Cmd/Ctrl or Shift starts picking
  // with only that row. Inside, a click toggles and Shift picks the range.
  const clickRow = (email: EmailSummary, intent: RowClick) => {
    if (!selection) onSelect(email)
    else if (!selection.active) {
      if (intent.toggle || intent.range) selection.start(email)
      else onSelect(email)
    } else if (intent.range) selection.extend(email, emails)
    else selection.toggle(email)
  }

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
      <ContextMenuPrimitive.Root
        disabled={menuDisabled}
        open={menu?.open ?? false}
        onOpenChange={(open) => setMenu((current) => current && { ...current, open })}
      >
        <ContextMenuPrimitive.Trigger
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
          onContextMenu={(event) => {
            // The native page menu stays blocked by the global guard; this only
            // decides whether the message menu opens, and for which row.
            const row = (event.target as HTMLElement).closest<HTMLElement>('[data-index]')
            const email = row ? emails[Number(row.dataset.index)] : undefined
            if (menuDisabled || !row || !email) {
              event.preventBaseUIHandler()
              return
            }
            if (anchorAtRow(event, row)) return
            const active = document.activeElement
            menuReturnFocus.current = active instanceof HTMLElement && active !== document.body ? active : null
            setMenu({ email, open: false })
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
                selectionMode={selection?.active ?? false}
                picked={selection?.isSelected(email.id) ?? false}
                menuOpen={email.id === menuOpenId}
                onSelect={(intent) => clickRow(email, intent)}
              />
            )
          })}
        </ContextMenuPrimitive.Trigger>
        {menu && (
          <EmailContextMenuContent
            email={menu.email}
            onOpen={onSelect}
            onRespond={onRespond}
            onFlag={onFlag}
            onArchive={onArchive}
            onDelete={onDelete}
            onRemoveLocal={onRemoveLocal}
            canArchive={canArchive}
            // Rows of a Sent view show recipients, not senders: no contact items there.
            onViewContact={sentView ? undefined : onViewContact}
            onAggregate={sentView ? undefined : onAggregate}
            isAggregated={isAggregated}
            viewingContact={viewingContact}
            finalFocus={() => {
              // Back to where focus was (the row, after Shift+F10 or the menu key);
              // never into the list just because the mouse was used.
              const target = menuReturnFocus.current
              menuReturnFocus.current = null
              return target?.isConnected ? target : false
            }}
          />
        )}
      </ContextMenuPrimitive.Root>
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

/** Menus re-sent at the row by `anchorAtRow`; they are never moved again. */
const reanchored = new WeakSet<Event>()

/**
 * Shift+F10 and the menu key send `contextmenu` with a point that may lie
 * outside the row, often (0, 0), and the menu opens at that point. Such an
 * event is replaced by one at the row: 16px in from its left edge, vertically
 * centred. A mouse right-click always lands inside the row and is left alone.
 * Returns true when the event was replaced.
 */
function anchorAtRow(event: BaseUIEvent<ReactMouseEvent<HTMLElement>>, row: HTMLElement) {
  if (reanchored.has(event.nativeEvent)) return false
  const rect = row.getBoundingClientRect()
  const { clientX: x, clientY: y } = event
  if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) return false
  const replay = new MouseEvent('contextmenu', {
    bubbles: true,
    cancelable: true,
    composed: true,
    button: 2,
    clientX: rect.left + 16,
    clientY: rect.top + rect.height / 2,
  })
  reanchored.add(replay)
  event.preventBaseUIHandler()
  event.preventDefault()
  event.target.dispatchEvent(replay)
  return true
}
