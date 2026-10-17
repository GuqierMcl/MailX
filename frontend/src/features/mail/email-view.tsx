import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Archive02Icon,
  Attachment01Icon,
  Delete02Icon,
  Forward01Icon,
  MailOpen01Icon,
  MailReply01Icon,
  MailReplyAll01Icon,
  StarIcon,
} from '@hugeicons/core-free-icons'
import type { EmailDetail, MailAddress } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { displayName, formatFullTime, formatSize, initials } from './format'

type EmailViewProps = {
  email: EmailDetail | null
  loading: boolean
  error: string | null
}

export function EmailView({ email, loading, error }: EmailViewProps) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <Toolbar disabled={!email} />
      <Separator />
      {loading ? (
        <div className="flex flex-col gap-3 p-4" role="status" aria-label="正在加载邮件">
          <Skeleton className="h-6 w-2/3" />
          <Skeleton className="h-4 w-1/3" />
          <Skeleton className="h-40 w-full" />
        </div>
      ) : error ? (
        <Empty>
          <EmptyHeader>
            <EmptyTitle>无法打开邮件</EmptyTitle>
            <EmptyDescription>{error}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : email ? (
        <EmailContent email={email} />
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

// Mail actions need the outbox and flag sync in core; until then the buttons
// only show where the actions will live.
const ACTIONS: [string, IconSvgElement][][] = [
  [
    ['归档', Archive02Icon],
    ['删除', Delete02Icon],
  ],
  [
    ['回复', MailReply01Icon],
    ['回复全部', MailReplyAll01Icon],
    ['转发', Forward01Icon],
  ],
  [['加星标', StarIcon]],
]

function Toolbar({ disabled }: { disabled: boolean }) {
  return (
    <div className="flex h-12 shrink-0 items-center gap-1 px-3" role="toolbar" aria-label="邮件操作">
      {ACTIONS.map((group, index) => (
        <div key={index} className="flex items-center gap-1">
          {index > 0 && <Separator orientation="vertical" className="mx-1 h-5" />}
          {group.map(([label, icon]) => (
            <Button key={label} variant="ghost" size="icon" aria-label={label} title={`${label}（尚未实现）`} disabled>
              <HugeiconsIcon icon={icon} />
            </Button>
          ))}
        </div>
      ))}
      <span className="sr-only">{disabled ? '未选择邮件' : '邮件操作尚未实现'}</span>
    </div>
  )
}

function addressList(addresses: MailAddress[]) {
  return addresses.map((address) => (address.name ? `${address.name} <${address.address}>` : address.address)).join('，')
}

function EmailContent({ email }: { email: EmailDetail }) {
  const { summary, attachments } = email
  return (
    <ScrollArea className="min-h-0 flex-1">
      <article className="flex flex-col">
        <header className="flex flex-col gap-4 p-4">
          <h2 className="text-lg font-semibold">{summary.subject}</h2>
          <div className="flex items-start gap-3 text-sm">
            <Avatar>
              <AvatarFallback>{initials(summary.from)}</AvatarFallback>
            </Avatar>
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
        <Separator />
        <div className="p-4 text-sm">
          <Body email={email} />
        </div>
        {attachments.length > 0 && (
          <>
            <Separator />
            <section className="flex flex-col gap-2 p-4" aria-label="附件">
              <h3 className="text-xs font-medium text-muted-foreground">{attachments.length} 个附件</h3>
              <ul className="flex flex-wrap gap-2">
                {attachments.map((attachment) => (
                  <li key={attachment.id} className="flex items-center gap-2 rounded-lg border px-3 py-2 text-xs">
                    <HugeiconsIcon icon={Attachment01Icon} className="size-3.5 text-muted-foreground" />
                    <span className="max-w-60 truncate">{attachment.filename}</span>
                    <span className="text-muted-foreground">{formatSize(attachment.size)}</span>
                  </li>
                ))}
              </ul>
            </section>
          </>
        )}
      </article>
    </ScrollArea>
  )
}

function Body({ email }: { email: EmailDetail }) {
  const { body, summary } = email
  if (!body) {
    return (
      <Alert>
        <AlertTitle>{summary.bodyState === 'failed' ? '正文获取失败' : '正文尚未下载'}</AlertTitle>
        <AlertDescription>正文会在连接邮箱后按需获取。</AlertDescription>
      </Alert>
    )
  }
  if (body.text) return <div className="whitespace-pre-wrap break-words">{body.text}</div>
  // HTML is untrusted and needs the isolated, script-free frame that is not built yet.
  return (
    <Alert>
      <AlertTitle>这封邮件只有 HTML 正文</AlertTitle>
      <AlertDescription>安全的 HTML 隔离显示尚未实现。</AlertDescription>
    </Alert>
  )
}
