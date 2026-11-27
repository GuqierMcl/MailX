import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { ContextMenu as ContextMenuPrimitive } from '@base-ui/react/context-menu'
import { defaultRangeExtractor, useVirtualizer, type Range } from '@tanstack/react-virtual'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowLeft01Icon, InboxIcon, Search01Icon } from '@hugeicons/core-free-icons'
import type { Account, EmailSummary, MailboxLabel, PreviewLines, WeekStartsOn } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'
import { anchorContextMenuAt } from '@/lib/context-menu-anchor'
import { cn } from '@/lib/utils'
import { dateGroupBoundaries, emailDateGroup, formatGroupedListTime, type DateGroup } from './date-groups'
import { EmailCard, type RowClick } from './email-card'
import { EmailContextMenuContent } from './email-context-menu'
import type { EmailMenuCallbacks, RespondKind } from './email-menu-actions'
import { SelectionHeader, type SelectionActions } from './selection-header'
import { escapeBelongsToSomethingElse } from './use-close-on-escape'
import type { ListSelection } from './use-list-selection'
import { useGaveUpVersion } from './preview-gave-up'
import { usePreviewRequests } from './use-preview-requests'
import type { VerificationCodes, VerificationTarget } from './use-verification-codes'
import type { OpenIntent } from '@/features/outbox/use-compose'

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

export type HistoryCoveragePrompt = {
  syncedAfter: string | null
  historyReachedEnd: boolean
  canLoadOlder: boolean
  loadingOlder?: boolean
  loadOlderError?: string | null
  onLoadOlder?: () => void
  onAdjustSyncRange?: () => void
}

type EmailListProps = Pick<EmailMenuCallbacks, 'onViewContact' | 'onAggregate' | 'isAggregated'> & {
  title: string
  /** The account owning this folder or local archive, shown beside the title. */
  accountName?: string
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
  mailboxLabel?: MailboxLabel
  groupByDate?: boolean
  weekStartsOn?: WeekStartsOn
  /** Asks for the previews of the listed rows; the complete wanted set every time. */
  onRequestPreviews?: (emailIds: string[]) => void
  verification?: VerificationCodes
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
  onRespond?: (email: EmailSummary, kind: RespondKind, intent?: OpenIntent) => void
  onFilterChange: (filter: ListFilter) => void
  onSelect: (email: EmailSummary) => void
  onLoadMore: () => void
  /** Multi-select; without it the list has no "Select" entry. */
  selection?: ListSelection
  /** What the selection header does with the picked messages. */
  selectionActions?: SelectionActions
  /** Scrolls this row into view; a new object each time it is asked (keyboard moves). */
  reveal?: RevealRequest | null
  historyCoverage?: HistoryCoveragePrompt
}

/** A request to bring a row into view; `seq` repeats a request for the same row. */
export type RevealRequest = { id: string; seq: number }

export function EmailList(props: EmailListProps) {
  useTranslation()
  const { title, accountName, contact, filter, onFilterChange, selection, selectionActions } = props
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
            {contact ? <ContactTitle contact={contact} /> : (
              <div className="flex min-w-0 flex-1 items-baseline gap-2">
                <h1 className="truncate text-base font-semibold">{title}</h1>
                {accountName && <span className="max-w-1/2 truncate text-xs font-normal text-muted-foreground">{accountName}</span>}
              </div>
            )}
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

function EmailRows({ contact, initialScrollTop, onScrollOffset, onViewContact, onAggregate, isAggregated, sentView = false, emails, accounts, loading, syncStatus, error, filter, search, indexedBodies = 0, selectedId, previewLines, mailboxLabel, groupByDate = true, weekStartsOn = 'monday', onRequestPreviews, verification, onFlag, onArchive, onDelete, onRemoveLocal, canArchive, onRespond, onSelect, hasMore, loadingMore, refreshing, stale, loadMoreError, onLoadMore, selection, reveal, historyCoverage }: EmailListProps) {
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
      sentView={sentView} emails={emails} accounts={accounts} stale={stale} selectedId={selectedId} previewLines={previewLines} mailboxLabel={mailboxLabel} onRequestPreviews={onRequestPreviews}
      groupByDate={groupByDate} weekStartsOn={weekStartsOn}
      onFlag={onFlag} onArchive={onArchive} onDelete={onDelete} onRemoveLocal={onRemoveLocal} canArchive={canArchive} onRespond={onRespond}
      hasMore={hasMore} loadingMore={loadingMore} refreshing={refreshing} loadMoreError={loadMoreError}
      windowCapped={syncStatus.windowCapped} searchHint={search ? t('mail:searchedBodies', { count: indexedBodies }) : null} onSelect={onSelect} onLoadMore={onLoadMore}
      selection={selection}
      verification={verification}
      viewingContact={contact?.address ?? null} onViewContact={onViewContact} onAggregate={onAggregate} isAggregated={isAggregated}
      initialScrollTop={initialScrollTop} onScrollOffset={onScrollOffset} reveal={reveal}
      historyCoverage={historyCoverage}
    />
  )
}

// Every row has the same height for a given preview setting (sender and subject are
// single-line, the preview slot is reserved), so the virtualizer can be told exactly.
// Row: py-3 (1.5rem) + sender line text-sm (1.25rem) + gap-1 (0.25rem) + subject
// line text-xs (1rem) = 4rem; with a preview, gap-2 (0.5rem) + one 1rem per line.
const PREVIEW_ROWS: Record<PreviewLines, number> = { none: 0, one: 1, two: 2, three: 3 }
const DATE_HEADING_REM = 1.25 // h-5
const ROW_GAP = 8
const LIST_PADDING = 12
const OVERSCAN = 8
/** Ask for the next page once the last visible row is this close to the end. */
const LOAD_MORE_THRESHOLD = 10

function rootPixels(rem: number) {
  return rem * (parseFloat(getComputedStyle(document.documentElement).fontSize) || 16)
}

const DATE_GROUP_LABELS = {
  today: 'mail:dateGroupToday',
  yesterday: 'mail:dateGroupYesterday',
  thisWeek: 'mail:dateGroupThisWeek',
  lastWeek: 'mail:dateGroupLastWeek',
  earlier: 'mail:dateGroupEarlier',
  future: 'mail:dateGroupFuture',
} as const satisfies Record<DateGroup, string>

type ListRow = { heading: DateGroup; key: string }
  | { email: EmailSummary; position: number; group: DateGroup | null; startsGroup: boolean; key: string }

function useDateBoundaries(weekStartsOn: WeekStartsOn) {
  const [now, setNow] = useState(() => new Date())
  useEffect(() => {
    let timeout: number
    function schedule(current: Date) {
      const { tomorrow } = dateGroupBoundaries(current, weekStartsOn)
      timeout = window.setTimeout(refresh, tomorrow - current.getTime())
    }
    function refresh() {
      const current = new Date()
      window.clearTimeout(timeout)
      setNow(current)
      schedule(current)
    }
    schedule(new Date())
    window.addEventListener('focus', refresh)
    document.addEventListener('visibilitychange', refresh)
    return () => {
      window.clearTimeout(timeout)
      window.removeEventListener('focus', refresh)
      document.removeEventListener('visibilitychange', refresh)
    }
  }, [weekStartsOn])
  return useMemo(() => dateGroupBoundaries(now, weekStartsOn), [now, weekStartsOn])
}

type VirtualRowsProps = {
  sentView: boolean
  emails: EmailSummary[]
  accounts: Account[] | null
  stale: boolean
  selectedId: string | null
  previewLines: PreviewLines
  mailboxLabel?: MailboxLabel
  groupByDate: boolean
  weekStartsOn: WeekStartsOn
  onRequestPreviews?: (emailIds: string[]) => void
  verification?: VerificationCodes
  onFlag?: (email: EmailSummary, flag: 'read' | 'starred', wanted: boolean) => void
  onArchive?: (email: EmailSummary, archive: boolean) => void
  onDelete?: (email: EmailSummary) => void
  onRemoveLocal?: (email: EmailSummary) => void
  canArchive?: (email: EmailSummary) => boolean
  onRespond?: (email: EmailSummary, kind: RespondKind, intent?: OpenIntent) => void
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
  reveal?: RevealRequest | null
  historyCoverage?: HistoryCoveragePrompt
} & Pick<EmailMenuCallbacks, 'onViewContact' | 'onAggregate' | 'isAggregated'>

function VirtualRows({ sentView, emails, accounts, stale, selectedId, previewLines, mailboxLabel, groupByDate, weekStartsOn, onRequestPreviews, verification, onFlag, onArchive, onDelete, onRemoveLocal, canArchive, onRespond, hasMore, loadingMore, refreshing, loadMoreError, windowCapped, searchHint, onSelect, onLoadMore, selection, viewingContact, onViewContact, onAggregate, isAggregated, initialScrollTop, onScrollOffset, reveal, historyCoverage }: VirtualRowsProps) {
  const { i18n } = useTranslation()
  const dates = useDateBoundaries(weekStartsOn)
  const boundaries = groupByDate ? dates : null
  const { rows, indexes: rowIndexes } = useMemo(() => {
    const rows: ListRow[] = []
    const indexes = new Map<string, number>()
    let previousGroup: DateGroup | null = null
    for (const [position, email] of emails.entries()) {
      const group = boundaries ? emailDateGroup(email.receivedAt, boundaries) : null
      const startsGroup = group !== null && group !== previousGroup
      if (startsGroup) rows.push({ heading: group, key: `date:${group}:${email.id}` })
      indexes.set(email.id, rows.length)
      rows.push({ email, position, group, startsGroup, key: email.id })
      previousGroup = group
    }
    return { rows, indexes }
  }, [emails, boundaries])
  const viewportRef = useRef<HTMLDivElement>(null)
  const searchHintRef = useRef<HTMLParagraphElement>(null)
  const dateLayout = groupByDate ? `${weekStartsOn}:${dates.today}` : ''
  const scrollAnchor = useRef<{ layout: string; id: string; index: number; position: number } | null>(null)
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
      const index = id === null ? undefined : rowIndexes.get(id)
      if (index !== undefined) indexes.push(index)
    }
    return indexes
  }, [rowIndexes, selectedId, focusedId, menuOpenId])
  const rangeExtractor = useCallback(
    (range: Range) => [...new Set([...defaultRangeExtractor(range), ...pinned])].sort((a, b) => a - b),
    [pinned],
  )
  const emailRem = 4 + (previewLines === 'none' ? 0 : 0.5 + PREVIEW_ROWS[previewLines])
  const estimateSize = (index: number) => rootPixels('heading' in rows[index] ? DATE_HEADING_REM : emailRem)
  // The React Compiler cannot memoize the virtualizer, which is fine: nothing here relies on it.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer<HTMLDivElement, HTMLDivElement>({
    count: rows.length,
    getScrollElement: () => viewportRef.current,
    getItemKey: (index) => rows[index].key,
    estimateSize,
    // A row that is not laid out (zero height) keeps the estimate instead of collapsing.
    measureElement: (element) => element.getBoundingClientRect().height || estimateSize(Number(element.dataset.index)),
    gap: ROW_GAP,
    paddingStart: LIST_PADDING,
    paddingEnd: LIST_PADDING,
    overscan: OVERSCAN,
    rangeExtractor,
    // Coming back to a list: start where the user left it.
    initialOffset: initialScrollTop,
  })

  // Keep the same visible message at the same offset when dates are regrouped.
  useLayoutEffect(() => {
    const viewport = viewportRef.current
    if (!viewport) return
    const previous = scrollAnchor.current
    let offset = viewport.scrollTop
    if (previous && previous.layout !== dateLayout && offset > 0) {
      const index = rowIndexes.get(previous.id)
      const row = index === undefined ? undefined : rows[index]
      if (index !== undefined && row && 'email' in row) {
        const mailDelta = row.position - previous.position
        const headingDelta = index - previous.index - mailDelta
        offset += mailDelta * (rootPixels(emailRem) + ROW_GAP) + headingDelta * (rootPixels(DATE_HEADING_REM) + ROW_GAP)
        virtualizer.scrollToOffset(offset)
      }
    }
    const localOffset = offset - (searchHintRef.current?.offsetHeight ?? 0)
    const first = virtualizer.getVirtualItemForOffset(Math.max(0, localOffset))
    let index = first ? first.index + Number(first.end <= localOffset) : 0
    if (rows[index] && 'heading' in rows[index]) index++
    const row = rows[index]
    scrollAnchor.current = row && 'email' in row ? { layout: dateLayout, id: row.email.id, index, position: row.position } : null
  })

  // A keyboard move brings its row into view, scrolling only as far as needed.
  // Only a new request scrolls; the list refreshing under it does not.
  const latestIndexes = useRef(rowIndexes)
  useEffect(() => {
    latestIndexes.current = rowIndexes
  }, [rowIndexes])
  useEffect(() => {
    if (!reveal) return
    const index = latestIndexes.current.get(reveal.id)
    if (index !== undefined) virtualizer.scrollToIndex(index, { align: 'auto' })
  }, [reveal, virtualizer])

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
  const lastVisibleRow = rows[lastVisible]
  const previousVisibleRow = rows[lastVisible - 1]
  const lastVisibleEmail = lastVisibleRow && 'email' in lastVisibleRow ? lastVisibleRow.position
    : previousVisibleRow && 'email' in previousVisibleRow ? previousVisibleRow.position : -1
  useEffect(() => {
    if (hasMore && !loadingMore && !refreshing && !loadMoreError && lastVisibleEmail >= emails.length - LOAD_MORE_THRESHOLD) onLoadMore()
  }, [hasMore, loadingMore, refreshing, loadMoreError, lastVisibleEmail, emails.length, onLoadMore])

  // Previews are fetched for the rows that are rendered (visible plus overscan).
  const virtualItems = virtualizer.getVirtualItems()
  const rendered = virtualItems.flatMap((item) => {
    const row = rows[item.index]
    return 'email' in row ? [row.email] : []
  })
  usePreviewRequests(onRequestPreviews, rendered, previewLines)
  useGaveUpVersion()
  // Unlike mounted overscan/pinned rows, only the actual viewport requests new
  // code recognition. A sender group is never treated as its representative mail.
  const codeTargetsKey = JSON.stringify(stale || selection?.active ? [] : virtualItems
    .filter((item) => item.index >= (virtualizer.range?.startIndex ?? 0) && item.index <= (virtualizer.range?.endIndex ?? -1))
    .flatMap((item) => {
      const row = rows[item.index]
      return 'email' in row ? [row.email] : []
    })
    .filter((email) => !email.senderGroup && !email.localOnly)
    .map((email) => ({ emailId: email.id, bodyState: email.bodyState })))
  const codeTargets = useMemo(() => JSON.parse(codeTargetsKey) as VerificationTarget[], [codeTargetsKey])
  const requestCodes = verification?.requestVisible
  useEffect(() => { requestCodes?.(codeTargets) }, [requestCodes, codeTargets])
  useEffect(() => () => { requestCodes?.([]) }, [requestCodes])

  // Outside selection mode a plain click opens; Cmd/Ctrl or Shift starts picking
  // with only that row. Inside, a click toggles and Shift picks the range.
  const clickRow = (email: EmailSummary, intent: RowClick) => {
    // A group row opens the sender's list; it is not a message, so it never opens in the
    // reader nor takes part in picking (picking mode ignores it).
    if (email.senderGroup) {
      if (!selection?.active) onViewContact?.(email)
      return
    }
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
        <p ref={searchHintRef} className="px-4 pt-3 text-xs text-muted-foreground">
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
            const element = (event.target as HTMLElement).closest<HTMLElement>('[data-index]')
            const row = element ? rows[Number(element.dataset.index)] : undefined
            setFocusedId(row && 'email' in row ? row.email.id : null)
          }}
          onBlur={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocusedId(null)
          }}
          onContextMenu={(event) => {
            // The native page menu stays blocked by the global guard; this only
            // decides whether the message menu opens, and for which row.
            const row = (event.target as HTMLElement).closest<HTMLElement>('[data-index]')
            const item = row ? rows[Number(row.dataset.index)] : undefined
            if (menuDisabled || !row || !item || 'heading' in item) {
              event.preventBaseUIHandler()
              return
            }
            if (anchorContextMenuAt(event, row)) return
            const active = document.activeElement
            menuReturnFocus.current = active instanceof HTMLElement && active !== document.body ? active : null
            setMenu({ email: item.email, open: false })
          }}
        >
          {virtualItems.map((item) => {
            const row = rows[item.index]
            const style = { transform: `translateY(${item.start}px)` }
            if ('heading' in row) {
              return (
                <div key={item.key} data-index={item.index} ref={virtualizer.measureElement} aria-hidden
                  data-slot="email-date-heading" style={style}
                  className="absolute inset-x-7 flex h-5 items-end text-xs font-medium text-muted-foreground">
                  {t(DATE_GROUP_LABELS[row.heading])}
                </div>
              )
            }
            const { email, position, group, startsGroup } = row
            return (
              <EmailCard
                key={item.key}
                ref={virtualizer.measureElement}
                index={item.index}
                position={position}
                count={emails.length}
                style={style}
                email={email}
                timeLabel={group && boundaries ? formatGroupedListTime(email.receivedAt, group, boundaries, i18n.resolvedLanguage ?? 'zh-CN') : undefined}
                dateGroupLabel={startsGroup && group ? t(DATE_GROUP_LABELS[group]) : undefined}
                sentView={sentView}
                account={accounts?.find((account) => account.id === email.accountId) ?? null}
                selected={!email.senderGroup && email.id === selectedId}
                previewLines={previewLines}
                mailboxLabel={mailboxLabel}
                onFlag={onFlag}
                onArchive={onArchive}
                onDelete={onDelete}
                onRemoveLocal={onRemoveLocal}
                canArchive={canArchive}
                verification={verification}
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
            // A group row is no single message: only its contact items are offered.
            onOpen={menu.email.senderGroup ? undefined : onSelect}
            onRespond={menu.email.senderGroup ? undefined : onRespond}
            onFlag={menu.email.senderGroup ? undefined : onFlag}
            onArchive={menu.email.senderGroup ? undefined : onArchive}
            onDelete={menu.email.senderGroup ? undefined : onDelete}
            onRemoveLocal={menu.email.senderGroup ? undefined : onRemoveLocal}
            canArchive={canArchive}
            // Rows of a Sent view show recipients, not senders: no contact items there.
            onViewContact={sentView ? undefined : onViewContact}
            onAggregate={sentView ? undefined : onAggregate}
            isAggregated={menu.email.senderGroup ? () => true : isAggregated}
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
      {!hasMore && historyCoverage && !historyCoverage.historyReachedEnd && historyCoverage.syncedAfter && (
        <div className="flex flex-col items-center gap-1 px-3 pb-3 text-center text-xs text-muted-foreground">
          <div className="flex flex-wrap items-center justify-center gap-x-2 gap-y-1">
            <span>{t('mail:syncRangeSyncedAfter', { date: historyCoverage.syncedAfter })}</span>
            {historyCoverage.canLoadOlder && historyCoverage.onLoadOlder && (
              <Button
                type="button"
                variant="link"
                size="xs"
                className="h-auto p-0 text-xs"
                disabled={historyCoverage.loadingOlder}
                onClick={historyCoverage.onLoadOlder}
              >
                {historyCoverage.loadingOlder ? t('mail:loadingOlder') : t('mail:loadOlder')}
              </Button>
            )}
            {historyCoverage.onAdjustSyncRange && (
              <Button
                type="button"
                variant="link"
                size="xs"
                className="h-auto p-0 text-xs"
                onClick={historyCoverage.onAdjustSyncRange}
              >
                {t('mail:adjustSyncRange')}
              </Button>
            )}
          </div>
          {historyCoverage.loadOlderError && (
            <p role="alert" className="text-destructive">
              {historyCoverage.loadOlderError}
            </p>
          )}
        </div>
      )}
      {windowCapped && !searchHint && (
        <p className="px-3 pb-3 text-center text-xs text-muted-foreground">{t('mail:syncCoverageLimited')}</p>
      )}
    </ScrollArea>
  )
}
