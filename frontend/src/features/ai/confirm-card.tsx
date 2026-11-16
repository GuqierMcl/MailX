import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Tick02Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import type { ConfirmationDetails, ConfirmationRequest, MailAddress } from '@/generated/accounts'
import { formatSize } from '@/features/mail/format'
import { cn } from '@/lib/utils'
import type { ConfirmStatus, SavedOutcome } from './chat-model'

/** Which words the card uses: the action, narrowed by what exactly it changes. */
type ConfirmKind = 'send' | 'retrySend' | 'cancelSend' | 'archive' | 'unarchive' | 'flags' | 'refresh' | 'draftCreate' | 'draftUpdate' | 'draftDelete'

function confirmKind({ details }: ConfirmationRequest): ConfirmKind {
  switch (details.type) {
    case 'send': return 'send'
    case 'outbox': return details.retry ? 'retrySend' : 'cancelSend'
    case 'organize': return details.change.type === 'flags' ? 'flags' : details.change.type
    case 'draft': return details.change.type === 'create' ? 'draftCreate' : details.change.type === 'delete' ? 'draftDelete' : 'draftUpdate'
  }
}

const address = (value: MailAddress) => value.name?.trim() ? `${value.name.trim()} <${value.address}>` : value.address

/** The facts of one request as label/value pairs, from the host's own data. */
function rows(details: ConfirmationDetails): { label: string; value: string; block?: boolean }[] {
  const field = (name: 'from' | 'to' | 'cc' | 'bcc' | 'subject' | 'body' | 'attachments' | 'emails' | 'draft' | 'change') => t(`ai:chat.confirm.field.${name}`)
  switch (details.type) {
    case 'send': return [
      { label: field('from'), value: details.accountName ? `${details.accountName} <${details.accountAddress}>` : details.accountAddress },
      { label: field('to'), value: details.to.map(address).join('、') },
      ...(details.cc.length > 0 ? [{ label: field('cc'), value: details.cc.map(address).join('、') }] : []),
      ...(details.bcc.length > 0 ? [{ label: field('bcc'), value: details.bcc.map(address).join('、') }] : []),
      { label: field('subject'), value: details.subject.trim() || t('ai:chat.confirm.noSubject') },
      ...(details.bodyPreview.trim() ? [{ label: field('body'), value: details.bodyPreview.trim(), block: true }] : []),
      ...(details.attachments.length > 0 ? [{ label: field('attachments'), value: details.attachments.map((file) => `${file.fileName} (${formatSize(file.size)})`).join('\n'), block: true }] : []),
    ]
    case 'outbox': return [
      { label: field('from'), value: details.senderAddress },
      { label: field('to'), value: details.recipients.join('、') + (details.recipientCount > details.recipients.length ? ` ${t('ai:chat.confirm.andMore', { count: details.recipientCount })}` : '') },
      { label: field('subject'), value: details.subject.trim() || t('ai:chat.confirm.noSubject') },
    ]
    case 'organize': {
      const subjects = details.subjects.map((subject) => subject.trim() || t('ai:chat.confirm.noSubject'))
      const more = details.count > subjects.length
      return [
        { label: field('emails'), value: t('ai:chat.confirm.emailCount', { count: details.count }) },
        ...(subjects.length > 0 ? [{ label: '', value: subjects.join('\n') + (more ? '\n…' : ''), block: true }] : []),
        ...(details.change.type === 'flags' ? [{ label: field('change'), value: flagChange(details.change.isRead, details.change.isStarred) }] : []),
      ]
    }
    case 'draft': return [
      { label: field('draft'), value: details.subject.trim() || t('ai:chat.confirm.noSubject') },
      { label: field('change'), value: draftChange(details.change) },
    ]
  }
}

function flagChange(isRead: boolean | null, isStarred: boolean | null) {
  const parts = [
    ...(isRead === null ? [] : [t(isRead ? 'ai:chat.confirm.change.read' : 'ai:chat.confirm.change.unread')]),
    ...(isStarred === null ? [] : [t(isStarred ? 'ai:chat.confirm.change.star' : 'ai:chat.confirm.change.unstar')]),
  ]
  return parts.join(t('ai:chat.confirm.change.separator'))
}

function draftField(name: string) {
  switch (name) {
    case 'to': return t('ai:chat.confirm.draftField.to')
    case 'cc': return t('ai:chat.confirm.draftField.cc')
    case 'bcc': return t('ai:chat.confirm.draftField.bcc')
    case 'subject': return t('ai:chat.confirm.draftField.subject')
    case 'body': case 'bodyHtml': case 'bodyText': return t('ai:chat.confirm.draftField.body')
    default: return name
  }
}

function draftChange(change: Extract<ConfirmationDetails, { type: 'draft' }>['change']) {
  switch (change.type) {
    case 'create': return t('ai:chat.confirm.change.create')
    case 'delete': return t('ai:chat.confirm.change.delete')
    case 'attachmentAdded': return t('ai:chat.confirm.change.attachmentAdded', { fileName: change.fileName })
    case 'attachmentRemoved': return t('ai:chat.confirm.change.attachmentRemoved', { fileName: change.fileName })
    case 'update': {
      const names = [...new Set(change.fields.map((name) => draftField(name)))]
      return t('ai:chat.confirm.change.update', { fields: names.join(t('ai:chat.confirm.change.separator')) })
    }
  }
}

function approvedText(request: ConfirmationRequest) {
  const key = request.details.type === 'send' ? 'send'
    : request.details.type === 'outbox' ? (request.details.retry ? 'retrySend' : 'cancelSend')
      : request.details.type === 'organize' ? 'organize' : 'draft'
  return t(`ai:chat.confirm.result.approved.${key}`)
}

/**
 * A request for the user's say before the assistant acts (ai-chat-ui.md 4.4). Neutral thin ring, no
 * colour but amber for risk lines, "取消" focused first. Once answered it folds in place to one line:
 * the details collapse (height and opacity) and the result fades in.
 */
export function ConfirmCard({ request, status, onRespond, onAnswered, cardRef }: {
  request: ConfirmationRequest
  status: ConfirmStatus
  onRespond: (approved: boolean) => void
  /** Called after the user answered, to give the focus back to the input. */
  onAnswered?: () => void
  cardRef?: (element: HTMLElement | null) => void
}) {
  useTranslation()
  const waiting = status === 'waiting'
  const kind = confirmKind(request)
  const cancel = useRef<HTMLButtonElement>(null)
  const title = t(`ai:chat.confirm.title.${kind}`)
  const warnings = request.details.type === 'send' ? request.details.warnings : []

  // It appears where the user may be typing: take the focus only from nothing, never from a text field.
  useEffect(() => {
    if (!waiting) return
    const active = document.activeElement
    if (active && active !== document.body && active.closest('input, textarea, [contenteditable]')) return
    cancel.current?.focus({ preventScroll: true })
  }, [waiting])

  const answer = (approved: boolean) => {
    onRespond(approved)
    onAnswered?.()
  }
  const result = status === 'approved' ? approvedText(request) : t('ai:chat.confirm.result.denied')
  return (
    <div
      ref={cardRef}
      role="group"
      aria-label={title}
      data-slot="confirm-card"
      data-status={status}
      className={cn(
        'relative rounded-xl text-sm transition-[padding,box-shadow] duration-200 ease-(--ease-out-soft) motion-reduce:transition-none',
        waiting ? 'p-4 ring-1 ring-foreground/10' : 'px-1 py-0 ring-1 ring-transparent',
      )}
    >
      {/* The details fold away: a grid row that shrinks to nothing, with the content fading as it goes. */}
      <div
        inert={!waiting}
        aria-hidden={!waiting}
        className={cn(
          'grid transition-[grid-template-rows,opacity] duration-200 ease-(--ease-out-soft) motion-reduce:transition-opacity motion-reduce:duration-100',
          waiting ? 'grid-rows-[1fr] opacity-100' : 'grid-rows-[0fr] opacity-0',
        )}
      >
        <div className="min-h-0 overflow-hidden">
          <p className="font-medium">{title}</p>
          <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
            {rows(request.details).map((row, index) => (
              <div key={index} className="contents">
                <dt className="text-muted-foreground">{row.label}</dt>
                <dd className={cn('min-w-0 wrap-anywhere', row.block && 'line-clamp-4 whitespace-pre-line text-muted-foreground', !row.label && 'col-span-2 col-start-1')}>{row.value}</dd>
              </div>
            ))}
          </dl>
          {warnings.length > 0 && (
            <ul className="mt-3 flex flex-col gap-0.5 text-xs text-amber-600 dark:text-amber-400">
              {warnings.map((warning) => <li key={warning}>{t(`ai:chat.confirm.warning.${warning}`)}</li>)}
            </ul>
          )}
          <div className="mt-4 flex justify-end gap-2">
            <Button ref={cancel} type="button" variant="ghost" size="lg" onClick={() => answer(false)}>{t('ai:chat.confirm.cancel')}</Button>
            <Button type="button" variant="default" size="lg" onClick={() => answer(true)}>{t(`ai:chat.confirm.action.${kind}`)}</Button>
          </div>
        </div>
      </div>
      <p
        role="status"
        className={cn(
          'flex items-center gap-1.5 text-muted-foreground transition-opacity duration-200 ease-(--ease-out-soft) motion-reduce:duration-100',
          waiting ? 'pointer-events-none absolute opacity-0' : 'py-1.5 opacity-100',
        )}
      >
        {!waiting && <>
          <HugeiconsIcon icon={Tick02Icon} aria-hidden className={cn('size-3.5', status !== 'approved' && 'invisible')} />
          {result}
        </>}
      </p>
    </div>
  )
}

/** What a saved confirmation says now: the answer in one line, in the card's own words (the details are not kept). */
export function ConfirmOutcomeLine({ outcome }: { outcome: SavedOutcome }) {
  useTranslation()
  const key = outcome.action === 'send' ? 'send' : outcome.action === 'cancelSend' ? 'cancelSend' : outcome.action === 'organize' ? 'organize' : 'draft'
  const result = outcome.outcome === 'approved' ? t(`ai:chat.confirm.result.approved.${key}`) : outcome.outcome === 'denied' ? t('ai:chat.confirm.result.denied') : t('ai:chat.confirm.result.expired')
  return (
    <p data-slot="confirm-card" data-status={outcome.outcome} className="flex items-center gap-1.5 px-1 py-1.5 text-sm text-muted-foreground">
      <HugeiconsIcon icon={Tick02Icon} aria-hidden className={cn('size-3.5', outcome.outcome !== 'approved' && 'invisible')} />
      {result}
    </p>
  )
}
