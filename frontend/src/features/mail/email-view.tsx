import { useState, type ReactNode, type Ref } from 'react'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Archive02Icon,
  ArchiveRestoreIcon,
  ArrowLeft01Icon,
  Cancel01Icon,
  Delete02Icon,
  Forward01Icon,
  MailOpen01Icon,
  MailReply01Icon,
  MailReplyAll01Icon,
  StarIcon,
} from '@hugeicons/core-free-icons'
import type { BodyFailureCode, EmailDetail, EmailSummary, MailAddress } from '@/generated/accounts'
import { Alert, AlertDescription } from '@/components/ui/alert'
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
import { ErrorNotice } from '@/components/error-notice'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'
import type { MailxClient } from '@/lib/mailx-client'
import { AttachmentList } from './attachment-list'
import { BodyDocument, BodySkeleton } from './body-document'
import { ImageDetailsSlot } from './image-status'
import { displayName, formatFullTime } from './format'
import { LocalOnlyMarker } from './local-only-marker'
import { SenderAvatar } from './sender-avatar'
import { ReadStatusButton } from './read-status-button'
import { useCloseOnEscape } from './use-close-on-escape'
import { ProviderAuthHelp } from '@/features/accounts/provider-auth-help'
import { TranslationBar } from './translation/translation-bar'
import { TranslationToolbarGroup } from './translation/translation-toolbar'
import type { EmailTranslation } from './translation/use-email-translation'
import { SummaryCard, SummaryToolbarButton } from './summary/summary-panel'
import type { EmailSummaryState } from './summary/use-email-summary'
import { VerificationCodeBar } from './verification-code-bar'
import type { VerificationCodes } from './use-verification-codes'
import { openIntent, type OpenIntent } from '@/features/outbox/use-compose'

export function RemoveLocalCopyDialog({ open, onOpenChange, onConfirm }: { open: boolean; onOpenChange: (open: boolean) => void; onConfirm: () => void }) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('mail:removeLocalCopyTitle')}</AlertDialogTitle>
          <AlertDialogDescription>{t('mail:removeLocalCopyDescription')}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('mail:cancel')}</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={onConfirm}>{t('mail:removeLocalCopyConfirm')}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

type EmailViewProps = {
  client: MailxClient
  /** Marks the open message read or starred; the list shows it at once. */
  onFlag: (email: EmailSummary, flag: 'read' | 'starred', wanted: boolean) => void
  /** Closes the reader. Nothing else closes it except opening another message. */
  onClose: () => void
  /** Single-pane readers return to the list, which restores focus to the message row. */
  backButton?: Ref<HTMLButtonElement>
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
  /** `intent` carries a Shift-click (open the other way this once). */
  onRespond: (kind: 'reply' | 'replyAll' | 'forward', intent?: OpenIntent) => void
  /** Drops a local-only sent copy from this device after the user confirmed; the server is untouched. */
  onRemoveLocal?: (email: EmailSummary) => void
  /** Deletes a server email (to Trash, or for good after confirming), with undo. */
  onDelete?: (email: EmailSummary) => void
  /** Archives (or, with `false`, restores) the email locally; set only where the email is in an inbox. */
  onArchive?: (email: EmailSummary, archive: boolean) => void
  /** Translation of the open message, shared with the command palette; absent where nothing can be translated. */
  translation?: EmailTranslation
  /** The AI summary of the open message; absent where the assistant cannot summarize. */
  aiSummary?: EmailSummaryState
  /** Opens the assistant's island about the open message ("继续问" on the summary). */
  onAskAi?: () => void
  /** Opens the AI settings, from a summary that cannot run. */
  onOpenAiSettings?: () => void
  verification?: VerificationCodes
  /** Wraps the reader's content (everything under the toolbar), e.g. with the rest of a conversation. */
  frame?: (content: ReactNode) => ReactNode
}

export function EmailView({ email, providerId, loading, error, client, downloading, onDownload, onFlag, onClose, backButton, onRespond, onRemoveLocal, onDelete, onArchive, hidden = false, frame, translation, verification, aiSummary, onAskAi, onOpenAiSettings }: EmailViewProps) {
  useTranslation()
  const showSkeleton = useDelayedFlag(loading)
  // A message is open while it is loading, shown, or failed to open. Only with
  // nothing open does the reader have no actions to offer.
  const open = Boolean(email) || loading || Boolean(error)
  useCloseOnEscape(open && !hidden, onClose)
  const content = loading || showSkeleton ? (
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
    <EmailContent email={email} providerId={providerId} client={client} downloading={downloading} onDownload={onDownload} translation={translation} verification={verification} aiSummary={aiSummary} onAskAi={onAskAi} onOpenAiSettings={onOpenAiSettings} />
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
  )
  return (
    <div className="flex h-full min-h-0 flex-col">
      {open && <Toolbar summary={email?.summary ?? null} onFlag={onFlag} onClose={onClose} backButton={backButton} onRespond={onRespond} onRemoveLocal={onRemoveLocal} onDelete={onDelete} onArchive={onArchive} translation={translation} aiSummary={aiSummary} />}
      {frame ? frame(content) : content}
    </div>
  )
}


const RESPOND_ACTIONS = [
  ['reply', 'mail:reply', MailReply01Icon],
  ['replyAll', 'mail:replyAll', MailReplyAll01Icon],
  ['forward', 'mail:forward', Forward01Icon],
] as const satisfies readonly (readonly ['reply' | 'replyAll' | 'forward', string, IconSvgElement])[]

function Toolbar({ summary, onFlag, onClose, backButton, onRespond, onRemoveLocal, onDelete, onArchive, translation, aiSummary }: { translation?: EmailTranslation; aiSummary?: EmailSummaryState; summary: EmailSummary | null; onFlag: EmailViewProps['onFlag']; onClose: () => void; backButton?: EmailViewProps['backButton']; onRespond: EmailViewProps['onRespond']; onRemoveLocal?: EmailViewProps['onRemoveLocal']; onDelete?: EmailViewProps['onDelete']; onArchive?: EmailViewProps['onArchive'] }) {
  useTranslation()
  const [confirmingRemove, setConfirmingRemove] = useState(false)
  // A local sent copy has no server message: it cannot be read-flagged, starred, archived or deleted remotely.
  const localOnly = summary?.localOnly === true
  return (
    <div className="flex h-12 shrink-0 items-center gap-1 px-3" role="toolbar" aria-label={t('mail:messageActions')}>
      {backButton && (
        <Button
          ref={backButton}
          variant="ghost"
          size="icon"
          aria-label={t('mail:backToMessageList')}
          title={t('mail:backToMessageList')}
          onClick={onClose}
        >
          <HugeiconsIcon icon={ArrowLeft01Icon} />
        </Button>
      )}
      <div className="flex items-center gap-1 not-first:ml-3">
        {localOnly ? (
          onRemoveLocal && (
            <IconButton label={t('mail:removeLocalCopy')} onClick={() => setConfirmingRemove(true)}>
              <HugeiconsIcon icon={Delete02Icon} />
            </IconButton>
          )
        ) : (
          <>
            {/* Local archive: only inbox mail has `onArchive`; the server is not touched. */}
            <IconButton
              label={summary?.archived ? t('mail:unarchive') : t('mail:archive')}
              tip={summary && !onArchive ? t('mail:archiveInboxOnly') : undefined}
              disabled={!summary || !onArchive}
              onClick={() => summary && onArchive?.(summary, !summary.archived)}
            >
              <HugeiconsIcon icon={summary?.archived ? ArchiveRestoreIcon : Archive02Icon} />
            </IconButton>
            <IconButton label={t('mail:delete')} disabled={!summary || !onDelete} onClick={() => summary && onDelete?.(summary)}>
              <HugeiconsIcon icon={Delete02Icon} />
            </IconButton>
          </>
        )}
      </div>
      <div className="flex items-center gap-1 not-first:ml-3">
        {RESPOND_ACTIONS.map(([kind, key, icon]) => (
          <IconButton key={kind} label={t(key)} disabled={!summary} onClick={(event) => onRespond(kind, openIntent(event))}>
            <HugeiconsIcon icon={icon} />
          </IconButton>
        ))}
      </div>
      {!localOnly && <div className="flex items-center gap-1 not-first:ml-3">
        <ReadStatusButton
          disabled={!summary}
          isRead={summary?.isRead ?? true}
          onClick={() => summary && onFlag(summary, 'read', !summary.isRead)}
        />
        <IconButton
          disabled={!summary}
          label={summary?.isStarred ? t('mail:unstar') : t('mail:star')}
          aria-pressed={summary?.isStarred ?? false}
          onClick={() => summary && onFlag(summary, 'starred', !summary.isStarred)}
          className={summary?.isStarred ? 'text-amber-500 hover:text-amber-500' : undefined}
        >
          <HugeiconsIcon icon={StarIcon} fill={summary?.isStarred ? 'currentColor' : 'none'} />
        </IconButton>
      </div>}
      {aiSummary?.available && summary && <SummaryToolbarButton summary={aiSummary} senderName={displayName(summary.from)} />}
      {translation?.available && !localOnly && <TranslationToolbarGroup translation={translation} />}
      {!backButton && <IconButton className="ml-auto" label={t('mail:closeMessage')} tip={t('mail:closeMessageShortcut')} onClick={onClose}>
        <HugeiconsIcon icon={Cancel01Icon} />
      </IconButton>}
      {summary && onRemoveLocal && (
        <RemoveLocalCopyDialog
          open={confirmingRemove}
          onOpenChange={setConfirmingRemove}
          onConfirm={() => onRemoveLocal(summary)}
        />
      )}
    </div>
  )
}

function addressList(addresses: MailAddress[]) {
  return addresses.map((address) => (address.name ? `${address.name} <${address.address}>` : address.address)).join('，')
}

type ContentProps = Pick<EmailViewProps, 'client' | 'downloading' | 'onDownload' | 'providerId' | 'translation' | 'verification' | 'aiSummary' | 'onAskAi' | 'onOpenAiSettings'> & { email: EmailDetail }

function EmailContent({ email, aiSummary, onAskAi, onOpenAiSettings, ...props }: ContentProps) {
  useTranslation()
  const [imageDetailsSlot, setImageDetailsSlot] = useState<HTMLSpanElement | null>(null)
  const { summary, attachments } = email
  return (
    <article className="flex min-h-0 flex-1 animate-in flex-col overflow-hidden duration-150 fade-in-0">
      <ScrollArea className="max-h-[35%] shrink-0">
        <header className="flex flex-col gap-4 p-4">
          <h2 className="text-lg font-semibold" title={props.translation?.subject ? summary.subject : undefined}>{props.translation?.subject ?? (summary.subject || t('mail:noSubject'))}</h2>
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
            {summary.localOnly && <LocalOnlyMarker />}
            <span ref={setImageDetailsSlot} className="shrink-0 empty:hidden" />
            <time dateTime={summary.receivedAt} className="shrink-0 text-xs text-muted-foreground">
              {formatFullTime(summary.receivedAt)}
            </time>
          </div>
        </header>
      </ScrollArea>
        <div className="flex min-h-0 flex-1 flex-col gap-2 p-4 text-sm">
          {aiSummary && <SummaryCard summary={aiSummary} client={props.client} onAsk={onAskAi} onOpenSettings={onOpenAiSettings ?? (() => undefined)} />}
          <VerificationCodeBar emailId={summary.id} verification={summary.localOnly ? undefined : props.verification} />
          <ImageDetailsSlot.Provider value={imageDetailsSlot}><Body email={email} {...props} /></ImageDetailsSlot.Provider>
        </div>
        {attachments.some((attachment) => !attachment.isInline) && <AttachmentList client={props.client} emailId={summary.id} attachments={attachments} />}
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

function Body({ email, providerId, client, downloading, onDownload, translation }: ContentProps) {
  useTranslation()
  const { body, summary, bodyFailure } = email
  if (downloading || (!body && (summary.bodyState === 'missing' || summary.bodyState === 'fetching'))) return <BodySkeleton />
  if (!body) return (
    <ErrorNotice layout="centered" message={bodyFailure ? BODY_FAILURE[bodyFailure] : t('mail:bodyFetchFailed')}
      actionLabel={t('mail:retry')} onAction={onDownload}>
      {bodyFailure === 'providerPolicy' && <ProviderAuthHelp className="max-w-sm shrink-0 items-center" client={client} providerId={providerId ?? null} reason="providerPolicy" />}
    </ErrorNotice>
  )
  return <div className="flex min-h-0 flex-1 flex-col gap-3">
    {body.truncated && <Alert><AlertDescription>{t('mail:bodyTruncated')}</AlertDescription></Alert>}
    {body.hasHtml
      ? <BodyDocument client={client} emailId={summary.id} hasRemoteImages={body.remoteImagesBlocked > 0}
        banner={translation?.available ? <TranslationBar translation={translation} /> : undefined}
        translated={translation?.available && translation.status?.translation ? {
          target: translation.status.targetLanguage, version: translation.generation, showing: translation.translationVisible, onFailed: translation.failBody,
        } : null} />
      : <PlainText text={body.text} />}
  </div>
}

function PlainText({ text }: { text: string | null }) {
  useTranslation()
  return <ScrollArea className="h-full min-h-0 flex-1"><div className="whitespace-pre-wrap break-words">{text || t('mail:bodyEmpty')}</div></ScrollArea>
}
