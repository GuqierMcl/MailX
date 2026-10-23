import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Archive02Icon,
  Cancel01Icon,
  Delete02Icon,
  Forward01Icon,
  Mail01Icon,
  MailOpen01Icon,
  MailReply01Icon,
  MailReplyAll01Icon,
  StarIcon,
} from '@hugeicons/core-free-icons'
import type { BodyFailureCode, EmailDetail, EmailSummary, MailAddress } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'
import type { MailxClient } from '@/lib/mailx-client'
import { AttachmentList } from './attachment-list'
import { BodyDocument, BodySkeleton } from './body-document'
import { displayName, formatFullTime } from './format'
import { SenderAvatar } from './sender-avatar'
import { useCloseOnEscape } from './use-close-on-escape'

type EmailViewProps = {
  client: MailxClient
  /** Marks the open message read or starred; the list shows it at once. */
  onFlag: (email: EmailSummary, flag: 'read' | 'starred', wanted: boolean) => void
  /** Closes the reader. Nothing else closes it except opening another message. */
  onClose: () => void
  downloading: boolean
  onDownload: () => void
  email: EmailDetail | null
  loading: boolean
  error: string | null
  /** The reader is folded away: keys must not close a message nobody can see. */
  hidden?: boolean
}

export function EmailView({ email, loading, error, client, downloading, onDownload, onFlag, onClose, hidden = false }: EmailViewProps) {
  const showSkeleton = useDelayedFlag(loading)
  // A message is open while it is loading, shown, or failed to open. Only with
  // nothing open does the reader have no actions to offer.
  const open = Boolean(email) || loading || Boolean(error)
  useCloseOnEscape(open && !hidden, onClose)
  return (
    <div className="flex h-full min-h-0 flex-col">
      {open && <Toolbar summary={email?.summary ?? null} onFlag={onFlag} onClose={onClose} />}
      {loading || showSkeleton ? (
        showSkeleton ? (
          <div className="flex animate-in flex-col gap-3 p-4 duration-150 fade-in-0" role="status" aria-label="正在加载邮件">
            <Skeleton className="h-6 w-2/3" />
            <Skeleton className="h-4 w-1/3" />
            <Skeleton className="h-40 w-full" />
          </div>
        ) : <div className="min-h-0 flex-1" />
      ) : error ? (
        <Empty>
          <EmptyHeader>
            <EmptyTitle>无法打开邮件</EmptyTitle>
            <EmptyDescription>{error}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : email ? (
        <EmailContent email={email} client={client} downloading={downloading} onDownload={onDownload} />
      ) : (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={MailOpen01Icon} />
            </EmptyMedia>
            <EmptyTitle>未选择邮件</EmptyTitle>
            <EmptyDescription>从左侧列表选择一封邮件查看内容。</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
    </div>
  )
}

// Reply, forward, archive and delete need the outbox and mailbox moves; until
// then those buttons only show where the actions will live.
const PLACEHOLDER_ACTIONS: [string, IconSvgElement][][] = [
  [
    ['归档', Archive02Icon],
    ['删除', Delete02Icon],
  ],
  [
    ['回复', MailReply01Icon],
    ['回复全部', MailReplyAll01Icon],
    ['转发', Forward01Icon],
  ],
]

function Toolbar({ summary, onFlag, onClose }: { summary: EmailSummary | null; onFlag: EmailViewProps['onFlag']; onClose: () => void }) {
  return (
    <div className="flex h-12 shrink-0 items-center gap-1 px-3" role="toolbar" aria-label="邮件操作">
      {PLACEHOLDER_ACTIONS.map((group, index) => (
        <div key={index} className="flex items-center gap-1 not-first:ml-3">
          {group.map(([label, icon]) => (
            <Button key={label} variant="ghost" size="icon" aria-label={label} title={`${label}（尚未实现）`} disabled>
              <HugeiconsIcon icon={icon} />
            </Button>
          ))}
        </div>
      ))}
      <div className="flex items-center gap-1 not-first:ml-3">
        <Button
          variant="ghost"
          size="icon"
          disabled={!summary}
          aria-label={summary?.isRead === false ? '标为已读' : '标为未读'}
          title={summary?.isRead === false ? '标为已读' : '标为未读'}
          onClick={() => summary && onFlag(summary, 'read', !summary.isRead)}
        >
          <HugeiconsIcon icon={summary?.isRead === false ? MailOpen01Icon : Mail01Icon} />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          disabled={!summary}
          aria-label={summary?.isStarred ? '取消星标' : '加星标'}
          aria-pressed={summary?.isStarred ?? false}
          title={summary?.isStarred ? '取消星标' : '加星标'}
          onClick={() => summary && onFlag(summary, 'starred', !summary.isStarred)}
          className={summary?.isStarred ? 'text-amber-500 hover:text-amber-500' : undefined}
        >
          <HugeiconsIcon icon={StarIcon} fill={summary?.isStarred ? 'currentColor' : 'none'} />
        </Button>
      </div>
      <Button
        variant="ghost"
        size="icon"
        className="ml-auto"
        aria-label="关闭邮件"
        title="关闭邮件（Esc）"
        onClick={onClose}
      >
        <HugeiconsIcon icon={Cancel01Icon} />
      </Button>
    </div>
  )
}

function addressList(addresses: MailAddress[]) {
  return addresses.map((address) => (address.name ? `${address.name} <${address.address}>` : address.address)).join('，')
}

type ContentProps = Pick<EmailViewProps, 'client' | 'downloading' | 'onDownload'> & { email: EmailDetail }

function EmailContent({ email, ...props }: ContentProps) {
  const { summary, attachments } = email
  return (
    <article className="flex min-h-0 flex-1 animate-in flex-col overflow-hidden duration-150 fade-in-0">
      <ScrollArea className="max-h-[35%] shrink-0">
        <header className="flex flex-col gap-4 p-4">
          <h2 className="text-lg font-semibold">{summary.subject || '（无主题）'}</h2>
          <div className="flex items-start gap-3 text-sm">
            <SenderAvatar sender={summary.from} accountId={summary.accountId} size="lg" />
            <div className="grid min-w-0 flex-1 gap-0.5">
              <div className="font-medium">
                {displayName(summary.from)}
                {summary.from.name && <span className="ml-1 font-normal text-muted-foreground">&lt;{summary.from.address}&gt;</span>}
              </div>
              {email.to.length > 0 && <div className="truncate text-xs text-muted-foreground">收件人：{addressList(email.to)}</div>}
              {email.cc.length > 0 && <div className="truncate text-xs text-muted-foreground">抄送：{addressList(email.cc)}</div>}
            </div>
            <time dateTime={summary.receivedAt} className="shrink-0 text-xs text-muted-foreground">
              {formatFullTime(summary.receivedAt)}
            </time>
          </div>
        </header>
      </ScrollArea>
        <div className="flex min-h-0 flex-1 flex-col p-4 text-sm">
          <Body email={email} {...props} />
        </div>
        {attachments.length > 0 && <AttachmentList client={props.client} emailId={summary.id} attachments={attachments} />}
    </article>
  )
}

const BODY_FAILURE: Record<BodyFailureCode, string> = {
  tooLarge: '邮件超过 100 MB，暂不支持下载',
  stale: '邮件已移动、删除或邮箱状态已变化，请刷新列表后重试。',
  network: '连接邮箱失败，请检查网络后重试。',
  authentication: '邮箱认证失败，请检查账号设置后重试。',
  protocol: '邮件下载或解析失败，请重试。',
  storage: '正文无法保存到本机，请检查可用空间后重试。',
}

function Body({ email, client, downloading, onDownload }: ContentProps) {
  const { body, summary, bodyFailure } = email
  if (downloading || (!body && (summary.bodyState === 'missing' || summary.bodyState === 'fetching'))) return <BodySkeleton />
  if (!body) return <Alert variant="destructive">
    <AlertTitle>正文获取失败</AlertTitle>
    <AlertDescription>{bodyFailure ? BODY_FAILURE[bodyFailure] : '未能获取正文，请重试。'}</AlertDescription>
    <Button className="mt-2" variant="outline" size="sm" onClick={onDownload}>重试</Button>
  </Alert>
  return <div className="flex min-h-0 flex-1 flex-col gap-3">
    {body.truncated && <Alert><AlertDescription>正文过长，当前仅显示截断后的内容。</AlertDescription></Alert>}
    {body.hasHtml
      ? <BodyDocument client={client} emailId={summary.id} hasRemoteImages={body.remoteImagesBlocked > 0} />
      : <PlainText text={body.text} />}
  </div>
}

function PlainText({ text }: { text: string | null }) {
  return <ScrollArea className="h-full min-h-0 flex-1"><div className="whitespace-pre-wrap break-words">{text || '这封邮件没有正文内容。'}</div></ScrollArea>
}
