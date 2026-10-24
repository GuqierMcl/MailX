import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { HugeiconsIcon } from '@hugeicons/react'
import { Delete02Icon, File01Icon, MailUpload01Icon, PencilEdit02Icon } from '@hugeicons/core-free-icons'
import type { Account, MailAddress, OutboxItem } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { IconButton } from '@/components/icon-button'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { toast } from '@/components/ui/toast'
import { formatFullTime } from '@/features/mail/format'
import { clientFailure, clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { composeKeys } from '@/lib/query-client'
import { errorSummary, recipientsPreview, STATE_LABELS } from './outbox-text'

const addressText = (list: MailAddress[]) => list.map((item) => (item.name ? `${item.name} <${item.address}>` : item.address)).join('，')

function Frame({ children, actions }: { children: React.ReactNode; actions: React.ReactNode }) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 px-3" role="toolbar" aria-label="操作">{actions}</div>
      <ScrollArea className="min-h-0 flex-1">{children}</ScrollArea>
    </div>
  )
}

/** The reader pane is not a flex column by itself; this gives `Empty` (flex-1) the height to centre in. */
function CenteredPane({ children }: { children: React.ReactNode }) {
  return <div data-slot="detail-empty" className="flex h-full min-h-0 flex-col">{children}</div>
}

function Placeholder({ icon, title, description }: { icon: typeof File01Icon; title: string; description: string }) {
  return (
    <CenteredPane>
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon"><HugeiconsIcon icon={icon} /></EmptyMedia>
          <EmptyTitle>{title}</EmptyTitle>
          <EmptyDescription>{description}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    </CenteredPane>
  )
}

export const DraftPlaceholder = () => <Placeholder icon={File01Icon} title="未选择草稿" description="从左侧列表选择一份草稿。" />
export const OutboxPlaceholder = () => <Placeholder icon={MailUpload01Icon} title="未选择邮件" description="从左侧列表选择一封邮件查看发送状态。" />

type DraftDetailProps = {
  client: MailxClient
  draftId: string
  accounts: Account[]
  onEdit: (draftId: string) => void
  /** The draft is gone (deleted here or sent elsewhere). */
  onGone: () => void
}

/** A saved draft: what it says so far, "继续编辑" (opens the composer) and 删除. */
export function DraftDetail({ client, draftId, accounts, onEdit, onGone }: DraftDetailProps) {
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const draft = useQuery({ queryKey: [...composeKeys.drafts(), draftId], queryFn: () => client.getDraft(draftId) })
  const remove = useMutation({
    mutationFn: () => client.deleteDraft(draftId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: composeKeys.drafts() })
      onGone()
    },
    onError: (cause) => {
      setConfirming(false)
      toast.error(clientCode(cause) === 'draftLocked' ? '这封邮件已在发送队列中，不能删除' : '无法删除草稿', {
        description: clientCode(cause) === 'draftLocked' ? undefined : clientFailure(cause).message,
      })
    },
  })

  // Deleted from the composer window or by the queue: drop the selection instead of showing "不存在".
  useEffect(() => client.onDrafts((change) => {
    if (change.draftId === draftId && change.change === 'deleted') onGone()
  }), [client, draftId, onGone])

  if (draft.isError) {
    return clientFailureCode(draft.error) === 'notFound'
      ? <Placeholder icon={File01Icon} title="这份草稿已不存在" description="它可能已被发送或删除。" />
      : <CenteredPane><Empty><EmptyHeader><EmptyTitle>无法打开草稿</EmptyTitle><EmptyDescription>{clientFailure(draft.error).message}</EmptyDescription></EmptyHeader></Empty></CenteredPane>
  }
  const data = draft.data
  const account = accounts.find((row) => row.id === data?.accountId)
  return (
    <Frame actions={(
      <>
        <IconButton label="继续编辑" disabled={!data} onClick={() => onEdit(draftId)}>
          <HugeiconsIcon icon={PencilEdit02Icon} />
        </IconButton>
        <IconButton label="删除" disabled={!data} onClick={() => setConfirming(true)}>
          <HugeiconsIcon icon={Delete02Icon} />
        </IconButton>
      </>
    )}>
      {data && (
        <article className="flex animate-in flex-col gap-4 p-4 duration-150 fade-in-0">
          <h2 className="text-lg font-semibold">{data.subject || '（无主题）'}</h2>
          <div className="grid gap-0.5 text-xs text-muted-foreground">
            {account && <div>发件人：{account.senderName ? `${account.senderName} <${account.emailAddress}>` : account.emailAddress}</div>}
            <div>收件人：{data.to.length ? addressText(data.to) : '（无）'}</div>
            {data.cc.length > 0 && <div>抄送：{addressText(data.cc)}</div>}
            {data.bcc.length > 0 && <div>密送：{addressText(data.bcc)}</div>}
            <div>最后修改：{formatFullTime(data.updatedAt)}</div>
          </div>
          <p className="text-sm break-words whitespace-pre-wrap">{data.bodyText || '（正文为空）'}</p>
        </article>
      )}
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>删除这份草稿？</AlertDialogTitle>
            <AlertDialogDescription>草稿会从这台电脑上删除，无法恢复。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction variant="destructive" disabled={remove.isPending} onClick={() => remove.mutate()}>删除</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Frame>
  )
}

const clientCode = clientFailureCode

type OutboxDetailProps = {
  client: MailxClient
  item: OutboxItem | null
  accounts: Account[]
  /** Opens the draft's composer (failed items: the draft is editable again). */
  onEdit: (draftId: string) => void
}

/** One unfinished send: status, who and what, why it stopped, and what can be done now. */
export function OutboxDetail({ client, item, accounts, onEdit }: OutboxDetailProps) {
  const queryClient = useQueryClient()
  const [confirmResend, setConfirmResend] = useState(false)
  const refresh = () => void queryClient.invalidateQueries({ queryKey: composeKeys.outbox() })
  const act = useMutation({
    mutationFn: (action: () => Promise<unknown>) => action(),
    onSuccess: refresh,
    onError: (cause) => {
      refresh()
      toast.error(clientCode(cause) === 'alreadySending' ? '已开始发送，无法撤销' : '操作没有完成', {
        description: clientCode(cause) === 'alreadySending' ? undefined : clientFailure(cause).message,
      })
    },
  })
  if (!item) return <OutboxPlaceholder />

  const account = accounts.find((row) => row.id === item.accountId)
  const canRetry = item.state === 'queued' && item.nextAttemptAt !== null
  const error = item.error
  const undo = () => act.mutate(async () => {
    await client.cancelOutbox(item.id)
    if (item.draftId) await client.openComposer(item.draftId)
  })
  const resolve = (resolution: 'markSent' | 'resend') => act.mutate(() => client.resolveUnknown({ id: item.id, resolution, clientMutationId: crypto.randomUUID() }))

  return (
    <Frame actions={(
      <>
        {item.state === 'queued' && <Button disabled={act.isPending} onClick={undo}>撤销</Button>}
        {canRetry && <Button variant="outline" disabled={act.isPending} onClick={() => act.mutate(() => client.retryOutboxNow(item.id))}>立即重试</Button>}
        {item.state === 'failed' && <Button disabled={!item.draftId} title={item.draftId ? undefined : '草稿已不存在'} onClick={() => item.draftId && onEdit(item.draftId)}>编辑并重新发送</Button>}
        {item.state === 'unknown' && (
          <>
            <Button disabled={act.isPending} onClick={() => resolve('markSent')}>标记为已发出</Button>
            <Button variant="outline" disabled={act.isPending} onClick={() => setConfirmResend(true)}>确认未发出，重新发送</Button>
          </>
        )}
      </>
    )}>
      <article className="flex animate-in flex-col gap-4 p-4 duration-150 fade-in-0">
        <h2 className="text-lg font-semibold">{item.subject || '（无主题）'}</h2>
        <div className="grid gap-0.5 text-xs text-muted-foreground">
          <div>状态：<span className="font-medium text-foreground">{STATE_LABELS[item.state]}</span></div>
          {account && <div>发件账号：{account.displayName}（{account.emailAddress}）</div>}
          <div>收件人：{recipientsPreview(item) || '（无）'}</div>
          <div>加入发送队列：{formatFullTime(item.firstQueuedAt)}</div>
          {item.attempts > 0 && <div>已尝试 {item.attempts} 次</div>}
        </div>
        {item.state === 'queued' && (
          <Alert role="status">
            <AlertTitle>{item.nextAttemptAt ? '稍后会自动重试' : '等待发送'}</AlertTitle>
            <AlertDescription>
              {item.nextAttemptAt
                ? `${error ? `上次未发出：${errorSummary(error)}。` : ''}下次尝试：${formatFullTime(item.nextAttemptAt)}。`
                : `预计发送时间：${formatFullTime(item.notBefore)}。在此之前可以撤销。`}
            </AlertDescription>
          </Alert>
        )}
        {item.state === 'sending' && <Alert role="status"><AlertDescription>正在连接发信服务器…这一步无法撤销。</AlertDescription></Alert>}
        {item.state === 'failed' && (
          <Alert variant="destructive" role="alert">
            <AlertTitle>邮件没有发出</AlertTitle>
            <AlertDescription>
              <p>{errorSummary(error)}</p>
              {error?.text && error.reason !== 'unknown' && <p className="mt-1 opacity-80">服务器回复{error.smtpCode ? `（${error.smtpCode}）` : ''}：{error.text}</p>}
              {error && error.rejected.length > 0 && (
                <ul aria-label="被拒绝的收件人" className="mt-2 grid gap-1">
                  {error.rejected.map((entry) => (
                    <li key={entry.address} className="break-words">
                      <span className="font-medium">{entry.address}</span>（{entry.code}）{entry.text}
                    </li>
                  ))}
                </ul>
              )}
              <p className="mt-2">可以修改后重新发送，草稿保持原样。</p>
            </AlertDescription>
          </Alert>
        )}
        {item.state === 'unknown' && (
          <Alert role="alert">
            <AlertTitle>服务器可能已收到这封邮件</AlertTitle>
            <AlertDescription>
              <p>发送时连接中断，没有收到服务器的最终回复，MailX 无法确定邮件是否已发出，也不会自动重发。</p>
              <p className="mt-1">请先到邮箱的已发送文件夹或收件人那里确认：已发出就选“标记为已发出”；确定没发出再选“重新发送”，重新发送沿用同一个 Message-ID，方便收件方去重。</p>
            </AlertDescription>
          </Alert>
        )}
      </article>
      <AlertDialog open={confirmResend} onOpenChange={setConfirmResend}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>确认这封邮件没有发出？</AlertDialogTitle>
            <AlertDialogDescription>如果服务器其实已经收到，收件人会收到两封相同的邮件。重新发送会沿用同一个 Message-ID。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => { setConfirmResend(false); resolve('resend') }}>确认未发出，重新发送</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Frame>
  )
}
