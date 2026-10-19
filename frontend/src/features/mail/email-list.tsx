import { HugeiconsIcon } from '@hugeicons/react'
import { Attachment01Icon, InboxIcon, Search01Icon, StarIcon } from '@hugeicons/core-free-icons'
import type { Account, EmailSummary } from '@/generated/accounts'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { cn } from '@/lib/utils'
import { displayName, formatListTime } from './format'

export type ListFilter = 'all' | 'unread'

export type ListSyncStatus = {
  phase: 'waiting' | 'syncing' | 'idle' | 'failed'
  failure: string | null
  windowCapped: boolean
}

type EmailListProps = {
  title: string
  emails: EmailSummary[]
  /** Shown as a badge on each row when the list spans several accounts. */
  accounts: Account[] | null
  loading: boolean
  syncStatus: ListSyncStatus
  hasMore: boolean
  loadingMore: boolean
  refreshing: boolean
  loadMoreError: string | null
  error: string | null
  filter: ListFilter
  search: string
  selectedId: string | null
  onFilterChange: (filter: ListFilter) => void
  onSearchChange: (search: string) => void
  onSelect: (email: EmailSummary) => void
  onLoadMore: () => void
}

export function EmailList(props: EmailListProps) {
  const { title, filter, search, onFilterChange, onSearchChange } = props
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
      <Separator />
      <div className="p-3">
        <div className="relative">
          <HugeiconsIcon
            icon={Search01Icon}
            className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            type="search"
            aria-label="搜索邮件"
            placeholder="搜索主题或发件人"
            className="pl-7"
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
          />
        </div>
      </div>
      <EmailRows {...props} />
    </Tabs>
  )
}

function EmailRows({ emails, accounts, loading, syncStatus, error, filter, search, selectedId, onSelect, hasMore, loadingMore, refreshing, loadMoreError, onLoadMore }: EmailListProps) {
  if (loading && emails.length === 0) {
    return (
      <div className="flex flex-col gap-2 px-3" role="status" aria-label="正在加载邮件">
        {Array.from({ length: 6 }, (_, index) => (
          <Skeleton key={index} className="h-20 w-full" />
        ))}
      </div>
    )
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
    <ScrollArea className="min-h-0 flex-1">
      {(syncStatus.phase !== 'idle' || syncStatus.windowCapped) && (
        <div role="status" aria-label="邮件同步状态" className="space-y-1 px-3 pb-3 text-xs text-muted-foreground">
          {syncStatus.phase !== 'idle' && <p>{syncTitle} {syncStatus.phase === 'failed' && syncDescription}</p>}
          {syncStatus.windowCapped && <p>同步覆盖受限：每轮最多同步窗口内最新的 1,000 封邮件头。</p>}
        </div>
      )}
      <div role="list" aria-label="邮件列表" className="flex flex-col gap-2 px-3 pb-3">
        {emails.map((email) => (
          <EmailRow
            key={email.id}
            email={email}
            account={accounts?.find((account) => account.id === email.accountId) ?? null}
            selected={email.id === selectedId}
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
    </ScrollArea>
  )
}

type EmailRowProps = { email: EmailSummary; account: Account | null; selected: boolean; onSelect: () => void }

function EmailRow({ email, account, selected, onSelect }: EmailRowProps) {
  return (
    <button
      type="button"
      role="listitem"
      aria-current={selected ? 'true' : undefined}
      onClick={onSelect}
      className={cn(
        'flex w-full flex-col items-start gap-2 rounded-lg border p-3 text-left text-sm transition-all hover:bg-accent',
        selected && 'bg-muted',
      )}
    >
      <div className="flex w-full flex-col gap-1">
        <div className="flex items-center gap-2">
          <span className={cn('truncate', email.isRead ? 'font-medium' : 'font-semibold')}>{displayName(email.from)}</span>
          {!email.isRead && <span aria-label="未读" className="size-2 shrink-0 rounded-full bg-primary" />}
          <span className={cn('ml-auto shrink-0 text-xs', selected ? 'text-foreground' : 'text-muted-foreground')}>
            {formatListTime(email.receivedAt)}
          </span>
        </div>
        <div className="flex items-center gap-1.5 text-xs font-medium">
          <span className="truncate">{email.subject || '（无主题）'}</span>
          {email.hasAttachment && (
            <HugeiconsIcon icon={Attachment01Icon} aria-label="有附件" className="size-3.5 shrink-0 text-muted-foreground" />
          )}
          {email.isStarred && <HugeiconsIcon icon={StarIcon} aria-label="已加星标" className="size-3.5 shrink-0 text-muted-foreground" />}
        </div>
      </div>
      {email.preview && <p className="line-clamp-2 text-xs text-muted-foreground">{email.preview}</p>}
      {account && <Badge variant="outline">{account.displayName}</Badge>}
    </button>
  )
}
