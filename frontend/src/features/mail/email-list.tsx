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
          <TabsTrigger value="all">全部</TabsTrigger>
          <TabsTrigger value="unread">未读</TabsTrigger>
        </TabsList>
      </div>
      <EmailRows {...props} />
    </Tabs>
  )
}

function EmailRows({ emails, accounts, loading, syncStatus, error, filter, search, selectedId, previewLines, onSelect, hasMore, loadingMore, refreshing, stale, loadMoreError, onLoadMore }: EmailListProps) {
  const pending = loading && emails.length === 0
  const showSkeleton = useDelayedFlag(pending)
  if (pending || showSkeleton) {
    // Nothing for the first moments: local data usually arrives before a
    // placeholder would be worth showing, and a flash is worse than a blink of blank.
    return showSkeleton ? (
      <div className="flex animate-in flex-col gap-2 px-3 pt-3 duration-150 fade-in-0" role="status" aria-label="正在加载邮件">
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
          <EmptyTitle>无法加载邮件</EmptyTitle>
          <EmptyDescription>{error}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }
  const syncTitle = {
    waiting: '等待同步邮件…',
    syncing: '正在同步邮件…',
    failed: '邮件同步失败',
    idle: '这里还没有邮件',
  }[syncStatus.phase]
  const syncDescription = syncStatus.phase === 'failed'
    ? syncStatus.failure ?? '稍后会自动重试，已有邮件仍可查看。'
    : syncStatus.phase === 'waiting'
      ? '轮到这个文件夹时会自动同步。'
      : '邮件头同步后会显示在这里。'
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
          {syncStatus.windowCapped && <EmptyDescription>同步覆盖受限：每轮最多同步窗口内最新的 1,000 封邮件头。</EmptyDescription>}
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
          <EmptyTitle>{search ? '没有匹配的邮件' : filter === 'unread' ? '没有未读邮件' : '这里还没有邮件'}</EmptyTitle>
          <EmptyDescription>{search ? '换个关键词试试。' : '新邮件同步后会显示在这里。'}</EmptyDescription>
          {syncStatus.phase !== 'idle' && <EmptyDescription>{syncTitle} {syncDescription}</EmptyDescription>}
          {syncStatus.windowCapped && <EmptyDescription>同步覆盖受限：每轮最多同步窗口内最新的 1,000 封邮件头。</EmptyDescription>}
        </EmptyHeader>
      </Empty>
    )
  }
  return (
    <ScrollArea className="min-h-0 flex-1 [&_[data-slot=scroll-area-viewport]]:!overflow-x-hidden [&_[data-slot=scroll-area-viewport]>div]:!block">
      <div
        role="list"
        aria-label="邮件列表"
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
            {loadingMore ? '正在加载…' : '加载更多'}
          </Button>
        </div>
      )}
      {syncStatus.windowCapped && (
        <p className="px-3 pb-3 text-center text-xs text-muted-foreground">同步覆盖受限：每轮最多同步窗口内最新的 1,000 封邮件头。</p>
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
        {!email.isRead && <span aria-label="未读" className="size-2 rounded-full bg-blue-500 dark:bg-blue-400" />}
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
              <HugeiconsIcon icon={StarIcon} aria-label="已加星标" fill="currentColor" className="size-3.5 shrink-0 text-amber-500" />
            )}
            <span className="truncate">{email.subject || '（无主题）'}</span>
            {email.hasAttachment && (
              <HugeiconsIcon icon={Attachment01Icon} aria-label="有附件" className="size-3.5 shrink-0 text-muted-foreground" />
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
