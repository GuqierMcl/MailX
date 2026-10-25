import { errorText } from '@/i18n/errors'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
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
import { clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { composeKeys } from '@/lib/query-client'
import { errorSummary, recipientsPreview, STATE_LABELS } from './outbox-text'

const addressText = (list: MailAddress[]) => list.map((item) => (item.name ? `${item.name} <${item.address}>` : item.address)).join('，')

function Frame({ children, actions }: { children: React.ReactNode; actions: React.ReactNode }) {
  useTranslation('composer')
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 px-3" role="toolbar" aria-label={t('composer:detail.actions')}>{actions}</div>
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

export function DraftPlaceholder() {
  useTranslation('composer')
  return <Placeholder icon={File01Icon} title={t('composer:detail.noDraft')} description={t('composer:detail.noDraftDescription')} />
}
export function OutboxPlaceholder() {
  useTranslation('composer')
  return <Placeholder icon={MailUpload01Icon} title={t('composer:detail.noMessage')} description={t('composer:detail.noMessageDescription')} />
}

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
  useTranslation('composer')
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
      toast.error(clientCode(cause) === 'draftLocked' ? t('composer:detail.lockedDelete') : t('composer:detail.deleteFailed'), {
        description: clientCode(cause) === 'draftLocked' ? undefined : errorText(cause),
      })
    },
  })

  // Deleted from the composer window or by the queue: drop the selection instead of showing "不存在".
  useEffect(() => client.onDrafts((change) => {
    if (change.draftId === draftId && change.change === 'deleted') onGone()
  }), [client, draftId, onGone])

  if (draft.isError) {
    return clientFailureCode(draft.error) === 'notFound'
      ? <Placeholder icon={File01Icon} title={t('composer:draft.gone')} description={t('composer:draft.goneDescription')} />
      : <CenteredPane><Empty><EmptyHeader><EmptyTitle>{t('composer:draft.openFailed')}</EmptyTitle><EmptyDescription>{errorText(draft.error)}</EmptyDescription></EmptyHeader></Empty></CenteredPane>
  }
  const data = draft.data
  const account = accounts.find((row) => row.id === data?.accountId)
  return (
    <Frame actions={(
      <>
        <IconButton label={t('composer:action.continueEditing')} disabled={!data} onClick={() => onEdit(draftId)}>
          <HugeiconsIcon icon={PencilEdit02Icon} />
        </IconButton>
        <IconButton label={t('composer:action.delete')} disabled={!data} onClick={() => setConfirming(true)}>
          <HugeiconsIcon icon={Delete02Icon} />
        </IconButton>
      </>
    )}>
      {data && (
        <article className="flex animate-in flex-col gap-4 p-4 duration-150 fade-in-0">
          <h2 className="text-lg font-semibold">{data.subject || t('composer:list.noSubject')}</h2>
          <div className="grid gap-0.5 text-xs text-muted-foreground">
            {account && <div>{t('composer:detail.from')}{account.senderName ? `${account.senderName} <${account.emailAddress}>` : account.emailAddress}</div>}
            <div>{t('composer:detail.to')}{data.to.length ? addressText(data.to) : t('composer:detail.none')}</div>
            {data.cc.length > 0 && <div>{t('composer:detail.cc')}{addressText(data.cc)}</div>}
            {data.bcc.length > 0 && <div>{t('composer:detail.bcc')}{addressText(data.bcc)}</div>}
            <div>{t('composer:detail.modified')}{formatFullTime(data.updatedAt)}</div>
          </div>
          <p className="text-sm break-words whitespace-pre-wrap">{data.bodyText || t('composer:detail.emptyBody')}</p>
        </article>
      )}
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('composer:detail.deleteTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t('composer:draft.deleteDescription')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('composer:action.cancel')}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" disabled={remove.isPending} onClick={() => remove.mutate()}>{t('composer:action.delete')}</AlertDialogAction>
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
  useTranslation('composer')
  const queryClient = useQueryClient()
  const [confirmResend, setConfirmResend] = useState(false)
  const refresh = () => void queryClient.invalidateQueries({ queryKey: composeKeys.outbox() })
  const act = useMutation({
    mutationFn: (action: () => Promise<unknown>) => action(),
    onSuccess: refresh,
    onError: (cause) => {
      refresh()
      toast.error(clientCode(cause) === 'alreadySending' ? t('composer:detail.alreadySending') : t('composer:detail.actionFailed'), {
        description: clientCode(cause) === 'alreadySending' ? undefined : errorText(cause),
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
        {item.state === 'queued' && <Button disabled={act.isPending} onClick={undo}>{t('composer:action.undo')}</Button>}
        {canRetry && <Button variant="outline" disabled={act.isPending} onClick={() => act.mutate(() => client.retryOutboxNow(item.id))}>{t('composer:detail.retryNow')}</Button>}
        {item.state === 'failed' && <Button disabled={!item.draftId} title={item.draftId ? undefined : t('composer:detail.draftGone')} onClick={() => item.draftId && onEdit(item.draftId)}>{t('composer:detail.editResend')}</Button>}
        {item.state === 'unknown' && (
          <>
            <Button disabled={act.isPending} onClick={() => resolve('markSent')}>{t('composer:detail.markSent')}</Button>
            <Button variant="outline" disabled={act.isPending} onClick={() => setConfirmResend(true)}>{t('composer:detail.resend')}</Button>
          </>
        )}
      </>
    )}>
      <article className="flex animate-in flex-col gap-4 p-4 duration-150 fade-in-0">
        <h2 className="text-lg font-semibold">{item.subject || t('composer:list.noSubject')}</h2>
        <div className="grid gap-0.5 text-xs text-muted-foreground">
          <div>{t('composer:detail.status')}<span className="font-medium text-foreground">{STATE_LABELS[item.state]}</span></div>
          {account && <div>{t('composer:detail.sendingAccount')}{account.displayName}（{account.emailAddress}）</div>}
          <div>{t('composer:detail.to')}{recipientsPreview(item) || t('composer:detail.none')}</div>
          <div>{t('composer:detail.queuedAt')}{formatFullTime(item.firstQueuedAt)}</div>
          {item.attempts > 0 && <div>{t('composer:detail.attempts', { count: item.attempts })}</div>}
        </div>
        {item.state === 'queued' && (
          <Alert role="status">
            <AlertTitle>{item.nextAttemptAt ? t('composer:detail.retryLater') : t('composer:detail.waiting')}</AlertTitle>
            <AlertDescription>
              {item.nextAttemptAt
                ? t('composer:detail.nextAttempt', { previous: error ? t('composer:detail.previousAttempt', { reason: errorSummary(error) }) : '', date: formatFullTime(item.nextAttemptAt) })
                : t('composer:detail.scheduledSend', { date: formatFullTime(item.notBefore) })}
            </AlertDescription>
          </Alert>
        )}
        {item.state === 'sending' && <Alert role="status"><AlertDescription>{t('composer:detail.connecting')}</AlertDescription></Alert>}
        {item.state === 'failed' && (
          <Alert variant="destructive" role="alert">
            <AlertTitle>{t('composer:detail.notSent')}</AlertTitle>
            <AlertDescription>
              <p>{errorSummary(error)}</p>
              {error?.text && error.reason !== 'unknown' && <p className="mt-1 opacity-80">{t('composer:detail.serverReply', { code: error.smtpCode ? `（${error.smtpCode}）` : '', text: error.text })}</p>}
              {error && error.rejected.length > 0 && (
                <ul aria-label={t('composer:detail.rejectedRecipients')} className="mt-2 grid gap-1">
                  {error.rejected.map((entry) => (
                    <li key={entry.address} className="break-words">
                      <span className="font-medium">{entry.address}</span>（{entry.code}）{entry.text}
                    </li>
                  ))}
                </ul>
              )}
              <p className="mt-2">{t('composer:detail.editRetryDescription')}</p>
            </AlertDescription>
          </Alert>
        )}
        {item.state === 'unknown' && (
          <Alert role="alert">
            <AlertTitle>{t('composer:detail.unknownTitle')}</AlertTitle>
            <AlertDescription>
              <p>{t('composer:detail.unknownDescription')}</p>
              <p className="mt-1">{t('composer:detail.unknownInstructions')}</p>
            </AlertDescription>
          </Alert>
        )}
      </article>
      <AlertDialog open={confirmResend} onOpenChange={setConfirmResend}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('composer:detail.resendTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t('composer:detail.resendDescription')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('composer:action.cancel')}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => { setConfirmResend(false); resolve('resend') }}>{t('composer:detail.resend')}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Frame>
  )
}
