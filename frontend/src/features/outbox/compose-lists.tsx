import { HugeiconsIcon } from '@hugeicons/react'
import { File01Icon, MailUpload01Icon } from '@hugeicons/core-free-icons'
import type { Account, DraftSummary, OutboxItem } from '@/generated/accounts'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import { formatListTime } from '@/features/mail/format'
import { recipientsPreview, STATE_LABELS } from './outbox-text'

type RowProps = { selected: boolean; onSelect: () => void; children: React.ReactNode }

function Row({ selected, onSelect, children }: RowProps) {
  return (
    <button
      type="button"
      role="listitem"
      aria-current={selected ? 'true' : undefined}
      onClick={onSelect}
      className={cn(
        'flex w-full flex-col items-start gap-1 rounded-lg px-3 py-3 text-left text-sm transition-colors hover:bg-foreground/5',
        selected && 'bg-background shadow-xs hover:bg-background',
      )}
    >
      {children}
    </button>
  )
}

function ListHeader({ title }: { title: string }) {
  return (
    <div className="flex h-12 shrink-0 items-center gap-2 px-3">
      <h1 className="truncate text-base font-semibold">{title}</h1>
    </div>
  )
}

/** The pane stays a full-height column so `Empty` (flex-1) can centre in what is left under the title. */
function EmptyShell({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div data-slot="list-empty" className="flex h-full min-h-0 flex-col">
      <ListHeader title={title} />
      {children}
    </div>
  )
}

function ListShell({ title, label, children }: { title: string; label: string; children: React.ReactNode }) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <ListHeader title={title} />
      <ScrollArea className="min-h-0 flex-1 [&_[data-slot=scroll-area-viewport]]:!overflow-x-hidden [&_[data-slot=scroll-area-viewport]>div]:!block">
        <div role="list" aria-label={label} className="flex animate-in flex-col gap-2 px-3 py-3 duration-150 fade-in-0">{children}</div>
      </ScrollArea>
    </div>
  )
}

const accountName = (accounts: Account[] | null, id: string) => accounts?.find((account) => account.id === id)?.displayName

type DraftListProps = {
  drafts: DraftSummary[] | undefined
  loading: boolean
  error: string | null
  accounts: Account[] | null
  selectedId: string | null
  onSelect: (id: string) => void
}

export function DraftList({ drafts, loading, error, accounts, selectedId, onSelect }: DraftListProps) {
  if (error) return <EmptyShell title="草稿"><Empty><EmptyHeader><EmptyTitle>无法加载草稿</EmptyTitle><EmptyDescription>{error}</EmptyDescription></EmptyHeader></Empty></EmptyShell>
  if (loading && !drafts) return <div className="min-h-0 flex-1" />
  if (!drafts || drafts.length === 0) {
    return (
      <EmptyShell title="草稿">
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon"><HugeiconsIcon icon={File01Icon} /></EmptyMedia>
            <EmptyTitle>没有草稿</EmptyTitle>
            <EmptyDescription>写到一半的邮件会自动保存在这里，只存在这台电脑上。</EmptyDescription>
          </EmptyHeader>
        </Empty>
      </EmptyShell>
    )
  }
  return (
    <ListShell title="草稿" label="草稿列表">
      {drafts.map((draft) => {
        const to = draft.to.map((item) => item.name || item.address).join('、')
        const account = accountName(accounts, draft.accountId)
        return (
          <Row key={draft.id} selected={draft.id === selectedId} onSelect={() => onSelect(draft.id)}>
            <div className="flex w-full items-center gap-2">
              <span className={cn('min-w-0 truncate', to ? 'font-semibold' : 'font-normal text-muted-foreground')}>{to || '（无收件人）'}</span>
              {account && <span className="min-w-0 shrink truncate text-xs text-muted-foreground">| {account}</span>}
              <span className="ml-auto shrink-0 text-xs text-muted-foreground">{formatListTime(draft.updatedAt)}</span>
            </div>
            <span className="w-full truncate text-xs text-foreground/75">{draft.subject || '（无主题）'}</span>
            {draft.preview && <span className="line-clamp-1 w-full text-xs text-muted-foreground">{draft.preview}</span>}
          </Row>
        )
      })}
    </ListShell>
  )
}

type OutboxListProps = {
  items: OutboxItem[] | undefined
  loading: boolean
  error: string | null
  accounts: Account[] | null
  selectedId: string | null
  onSelect: (id: string) => void
}

const STATE_TONE: Record<string, string> = {
  failed: 'text-destructive',
  unknown: 'text-amber-600 dark:text-amber-400',
}

export function OutboxList({ items, loading, error, accounts, selectedId, onSelect }: OutboxListProps) {
  if (error) return <EmptyShell title="发件箱"><Empty><EmptyHeader><EmptyTitle>无法加载发件箱</EmptyTitle><EmptyDescription>{error}</EmptyDescription></EmptyHeader></Empty></EmptyShell>
  if (loading && !items) return <div className="min-h-0 flex-1" />
  if (!items || items.length === 0) {
    return (
      <EmptyShell title="发件箱">
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon"><HugeiconsIcon icon={MailUpload01Icon} /></EmptyMedia>
            <EmptyTitle>发件箱是空的</EmptyTitle>
            <EmptyDescription>正在发送、发送失败或需要确认的邮件会出现在这里。</EmptyDescription>
          </EmptyHeader>
        </Empty>
      </EmptyShell>
    )
  }
  return (
    <ListShell title="发件箱" label="发件箱列表">
      {items.map((item) => {
        const account = accountName(accounts, item.accountId)
        return (
          <Row key={item.id} selected={item.id === selectedId} onSelect={() => onSelect(item.id)}>
            <div className="flex w-full items-center gap-2">
              <span className="min-w-0 truncate font-semibold">{recipientsPreview(item) || '（无收件人）'}</span>
              {account && <span className="min-w-0 shrink truncate text-xs text-muted-foreground">| {account}</span>}
              <span className={cn('ml-auto shrink-0 text-xs font-medium', STATE_TONE[item.state] ?? 'text-muted-foreground')}>{STATE_LABELS[item.state]}</span>
            </div>
            <span className="w-full truncate text-xs text-foreground/75">{item.subject || '（无主题）'}</span>
          </Row>
        )
      })}
    </ListShell>
  )
}
