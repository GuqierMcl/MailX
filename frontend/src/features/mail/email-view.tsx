import { useState } from 'react'
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
import type { BodyFailureCode, EmailDetail, MailAddress } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import type { MailxClient } from '@/lib/mailx-client'
import { BodyDocument, BodySkeleton } from './body-document'
import { displayName, formatFullTime, formatSize, initials } from './format'

type EmailViewProps = {
  client: MailxClient
  downloading: boolean
  onDownload: () => void
  email: EmailDetail | null
  loading: boolean
  error: string | null
}

export function EmailView({ email, loading, error, client, downloading, onDownload }: EmailViewProps) {
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

type ContentProps = Pick<EmailViewProps, 'client' | 'downloading' | 'onDownload'> & { email: EmailDetail }

function EmailContent({ email, ...props }: ContentProps) {
  const { summary, attachments } = email
  return (
    <article className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <ScrollArea className="max-h-[35%] shrink-0">
        <header className="flex flex-col gap-4 p-4">
          <h2 className="text-lg font-semibold">{summary.subject || '（无主题）'}</h2>
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
      </ScrollArea>
        <Separator />
        <div className="flex min-h-0 flex-1 flex-col p-4 text-sm">
          <Body email={email} {...props} />
        </div>
        {attachments.length > 0 && (
          <>
            <Separator />
            <section className="flex max-h-36 shrink-0 flex-col gap-2 overflow-auto p-4" aria-label="附件">
              <h3 className="text-xs font-medium text-muted-foreground">{attachments.length} 个附件</h3>
              <ul className="flex flex-wrap gap-2">
                {attachments.map((attachment) => (
                  <li key={attachment.id} className="flex items-center gap-2 rounded-lg border px-3 py-2 text-xs">
                    <HugeiconsIcon icon={Attachment01Icon} className="size-3.5 text-muted-foreground" />
                    <span className="max-w-60 break-all">{attachment.filename}</span>
                    <span className="text-muted-foreground">{attachment.mimeType}</span>
                    <span className="text-muted-foreground">{formatSize(attachment.size)}</span>
                  </li>
                ))}
              </ul>
            </section>
          </>
        )}
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
  const [tab, setTab] = useState<string>('html')
  const { body, summary, bodyFailure } = email
  if (downloading || (!body && (summary.bodyState === 'missing' || summary.bodyState === 'fetching'))) return <BodySkeleton />
  if (!body) return <Alert variant="destructive">
    <AlertTitle>正文获取失败</AlertTitle>
    <AlertDescription>{bodyFailure ? BODY_FAILURE[bodyFailure] : '未能获取正文，请重试。'}</AlertDescription>
    <Button className="mt-2" variant="outline" size="sm" onClick={onDownload}>重试</Button>
  </Alert>
  return <div className="flex min-h-0 flex-1 flex-col gap-3">
    {body.remoteImagesBlocked > 0 && <Alert><AlertDescription>已阻止 {body.remoteImagesBlocked} 张远程图片</AlertDescription></Alert>}
    {body.truncated && <Alert><AlertDescription>正文过长，当前仅显示截断后的内容。</AlertDescription></Alert>}
    {body.hasHtml ? <Tabs value={tab} onValueChange={(value) => setTab(String(value))} className="min-h-0 flex-1">
      <TabsList aria-label="正文格式"><TabsTrigger value="html">HTML</TabsTrigger><TabsTrigger value="text">纯文本</TabsTrigger></TabsList>
      <TabsContent value="html" className="flex min-h-0 flex-col">
        {tab === 'html' && <BodyDocument client={client} emailId={summary.id} />}
      </TabsContent>
      <TabsContent value="text" className="min-h-0 overflow-hidden"><PlainText text={body.text} /></TabsContent>
    </Tabs> : <PlainText text={body.text} />}
  </div>
}

function PlainText({ text }: { text: string | null }) {
  return <ScrollArea className="h-full min-h-0 flex-1"><div className="whitespace-pre-wrap break-words">{text || '这封邮件没有正文内容。'}</div></ScrollArea>
}
