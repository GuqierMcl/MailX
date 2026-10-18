import { HugeiconsIcon } from '@hugeicons/react'
import { Attachment01Icon, InboxIcon, Search01Icon, StarIcon } from '@hugeicons/core-free-icons'
import type { Account, EmailSummary } from '@/generated/accounts'
import { Badge } from '@/components/ui/badge'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { cn } from '@/lib/utils'
import { displayName, formatListTime } from './format'

export type ListFilter = 'all' | 'unread'

type EmailListProps = {
  title: string
  emails: EmailSummary[]
  /** Shown as a badge on each row when the list spans several accounts. */
  accounts: Account[] | null
  loading: boolean
  /** Nothing is cached for this view yet because messages are not synced. */
  unsynced: boolean
  error: string | null
  filter: ListFilter
  search: string
  selectedId: string | null
  onFilterChange: (filter: ListFilter) => void
  onSearchChange: (search: string) => void
  onSelect: (email: EmailSummary) => void
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
            placeholder="搜索主题、发件人或内容"
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

function EmailRows({ emails, accounts, loading, unsynced, error, filter, search, selectedId, onSelect }: EmailListProps) {
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
  if (unsynced) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={InboxIcon} />
          </EmptyMedia>
          <EmptyTitle>邮件尚未同步</EmptyTitle>
          <EmptyDescription>当前版本只同步文件夹列表，邮件同步将在后续版本提供。</EmptyDescription>
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
        </EmptyHeader>
      </Empty>
    )
  }
  return (
    <ScrollArea className="min-h-0 flex-1">
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
          <span className="truncate">{email.subject}</span>
          {email.hasAttachment && (
            <HugeiconsIcon icon={Attachment01Icon} aria-label="有附件" className="size-3.5 shrink-0 text-muted-foreground" />
          )}
          {email.isStarred && <HugeiconsIcon icon={StarIcon} aria-label="已加星标" className="size-3.5 shrink-0 text-muted-foreground" />}
        </div>
      </div>
      <p className="line-clamp-2 text-xs text-muted-foreground">{email.preview}</p>
      {account && <Badge variant="outline">{account.displayName}</Badge>}
    </button>
  )
}
