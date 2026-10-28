import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
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
import { IconButton } from '@/components/icon-button'
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
import { ProviderAuthHelp } from '@/features/accounts/provider-auth-help'

type EmailViewProps = {
  client: MailxClient
  /** Marks the open message read or starred; the list shows it at once. */
  onFlag: (email: EmailSummary, flag: 'read' | 'starred', wanted: boolean) => void
  /** Closes the reader. Nothing else closes it except opening another message. */
  onClose: () => void
  downloading: boolean
  onDownload: () => void
  email: EmailDetail | null
  /** The owning account's saved provider; mail content cannot choose its help link. */
  providerId?: string | null
  loading: boolean
  error: string | null
  /** The reader is folded away: keys must not close a message nobody can see. */
  hidden?: boolean
  /** Starts a reply, reply-all or forward of the open message (opens the composer window). */
  onRespond: (kind: 'reply' | 'replyAll' | 'forward') => void
}

export function EmailView({ email, providerId, loading, error, client, downloading, onDownload, onFlag, onClose, onRespond, hidden = false }: EmailViewProps) {
  useTranslation()
  const showSkeleton = useDelayedFlag(loading)
  // A message is open while it is loading, shown, or failed to open. Only with
  // nothing open does the reader have no actions to offer.
  const open = Boolean(email) || loading || Boolean(error)
  useCloseOnEscape(open && !hidden, onClose)
  return (
    <div className="flex h-full min-h-0 flex-col">
      {open && <Toolbar summary={email?.summary ?? null} onFlag={onFlag} onClose={onClose} onRespond={onRespond} />}
      {loading || showSkeleton ? (
        showSkeleton ? (
          <div className="flex animate-in flex-col gap-3 p-4 duration-150 fade-in-0" role="status" aria-label={t('mail:loadingMail')}>
            <Skeleton className="h-6 w-2/3" />
            <Skeleton className="h-4 w-1/3" />
            <Skeleton className="h-40 w-full" />
          </div>
        ) : <div className="min-h-0 flex-1" />
      ) : error ? (
        <Empty>
          <EmptyHeader>
            <EmptyTitle>{t('mail:openMailFailed')}</EmptyTitle>
            <EmptyDescription>{error}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : email ? (
        <EmailContent email={email} providerId={providerId} client={client} downloading={downloading} onDownload={onDownload} />
      ) : (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={MailOpen01Icon} />
            </EmptyMedia>
            <EmptyTitle>{t('mail:noMessageSelected')}</EmptyTitle>
            <EmptyDescription>{t('mail:selectMessageHint')}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
    </div>
  )
}

// Archive and delete need mailbox moves; until then those buttons only show
// where the actions will live.
const PLACEHOLDER_ACTIONS = [
  ['mail:archive', Archive02Icon],
  ['mail:delete', Delete02Icon],
] as const satisfies readonly (readonly [string, IconSvgElement])[]

const RESPOND_ACTIONS = [
  ['reply', 'mail:reply', MailReply01Icon],
  ['replyAll', 'mail:replyAll', MailReplyAll01Icon],
  ['forward', 'mail:forward', Forward01Icon],
] as const satisfies readonly (readonly ['reply' | 'replyAll' | 'forward', string, IconSvgElement])[]

function Toolbar({ summary, onFlag, onClose, onRespond }: { summary: EmailSummary | null; onFlag: EmailViewProps['onFlag']; onClose: () => void; onRespond: EmailViewProps['onRespond'] }) {
  useTranslation()
  return (
    <div className="flex h-12 shrink-0 items-center gap-1 px-3" role="toolbar" aria-label={t('mail:messageActions')}>
      <div className="flex items-center gap-1 not-first:ml-3">
        {PLACEHOLDER_ACTIONS.map(([key, icon]) => (
          <IconButton key={key} label={t(key)} tip={t('mail:notImplemented', { label: t(key) })} disabled>
            <HugeiconsIcon icon={icon} />
          </IconButton>
        ))}
      </div>
      <div className="flex items-center gap-1 not-first:ml-3">
        {RESPOND_ACTIONS.map(([kind, key, icon]) => (
          <IconButton key={kind} label={t(key)} disabled={!summary} onClick={() => onRespond(kind)}>
            <HugeiconsIcon icon={icon} />
          </IconButton>
        ))}
      </div>
      <div className="flex items-center gap-1 not-first:ml-3">
        <IconButton
          disabled={!summary}
          label={summary?.isRead === false ? t('mail:markRead') : t('mail:markUnread')}
          onClick={() => summary && onFlag(summary, 'read', !summary.isRead)}
        >
          <HugeiconsIcon icon={summary?.isRead === false ? MailOpen01Icon : Mail01Icon} />
        </IconButton>
        <IconButton
          disabled={!summary}
          label={summary?.isStarred ? t('mail:unstar') : t('mail:star')}
          aria-pressed={summary?.isStarred ?? false}
          onClick={() => summary && onFlag(summary, 'starred', !summary.isStarred)}
          className={summary?.isStarred ? 'text-amber-500 hover:text-amber-500' : undefined}
        >
          <HugeiconsIcon icon={StarIcon} fill={summary?.isStarred ? 'currentColor' : 'none'} />
        </IconButton>
      </div>
      <IconButton className="ml-auto" label={t('mail:closeMessage')} tip={t('mail:closeMessageShortcut')} onClick={onClose}>
        <HugeiconsIcon icon={Cancel01Icon} />
      </IconButton>
    </div>
  )
}

function addressList(addresses: MailAddress[]) {
  return addresses.map((address) => (address.name ? `${address.name} <${address.address}>` : address.address)).join('，')
}

type ContentProps = Pick<EmailViewProps, 'client' | 'downloading' | 'onDownload' | 'providerId'> & { email: EmailDetail }

function EmailContent({ email, ...props }: ContentProps) {
  useTranslation()
  const { summary, attachments } = email
  return (
    <article className="flex min-h-0 flex-1 animate-in flex-col overflow-hidden duration-150 fade-in-0">
      <ScrollArea className="max-h-[35%] shrink-0">
        <header className="flex flex-col gap-4 p-4">
          <h2 className="text-lg font-semibold">{summary.subject || t('mail:noSubject')}</h2>
          <div className="flex items-start gap-3 text-sm">
            <SenderAvatar sender={summary.from} accountId={summary.accountId} size="lg" />
            <div className="grid min-w-0 flex-1 gap-0.5">
              <div className="font-medium">
                {displayName(summary.from)}
                {summary.from.name && <span className="ml-1 font-normal text-muted-foreground">&lt;{summary.from.address}&gt;</span>}
              </div>
              {email.to.length > 0 && <div className="truncate text-xs text-muted-foreground">{t('mail:toLabel')}{addressList(email.to)}</div>}
              {email.cc.length > 0 && <div className="truncate text-xs text-muted-foreground">{t('mail:ccLabel')}{addressList(email.cc)}</div>}
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
  get tooLarge() { return t('mail:messageTooLarge') },
  get stale() { return t('mail:messageMovedOrDeleted') },
  get network() { return t('mail:mailConnectionFailed') },
  get authentication() { return t('mail:mailAuthenticationFailed') },
  get providerPolicy() { return t('mail:mailProviderPolicyFailed') },
  get protocol() { return t('mail:mailDownloadFailed') },
  get storage() { return t('mail:bodyStorageFailed') },
}

function Body({ email, providerId, client, downloading, onDownload }: ContentProps) {
  useTranslation()
  const { body, summary, bodyFailure } = email
  if (downloading || (!body && (summary.bodyState === 'missing' || summary.bodyState === 'fetching'))) return <BodySkeleton />
  if (!body) return <Alert variant="destructive">
    <AlertTitle>{t('mail:bodyFetchFailed')}</AlertTitle>
    <AlertDescription>{bodyFailure ? BODY_FAILURE[bodyFailure] : t('mail:retryBodyHint')}</AlertDescription>
    {bodyFailure === 'providerPolicy' && <ProviderAuthHelp client={client} providerId={providerId ?? null} reason="providerPolicy" />}
    <Button className="mt-2" variant="outline" size="sm" onClick={onDownload}>{t('mail:retry')}</Button>
  </Alert>
  return <div className="flex min-h-0 flex-1 flex-col gap-3">
    {body.truncated && <Alert><AlertDescription>{t('mail:bodyTruncated')}</AlertDescription></Alert>}
    {body.hasHtml
      ? <BodyDocument client={client} emailId={summary.id} hasRemoteImages={body.remoteImagesBlocked > 0} />
      : <PlainText text={body.text} />}
  </div>
}

function PlainText({ text }: { text: string | null }) {
  useTranslation()
  return <ScrollArea className="h-full min-h-0 flex-1"><div className="whitespace-pre-wrap break-words">{text || t('mail:bodyEmpty')}</div></ScrollArea>
}
