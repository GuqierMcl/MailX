import { errorText } from '@/i18n/errors'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { useEffect, useImperativeHandle, useMemo, useState, type Ref } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { AlertCircleIcon, MailOpen01Icon } from '@hugeicons/core-free-icons'
import type { Account } from '@/generated/accounts'
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
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { toast } from '@/components/ui/toast'
import { clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { RecipientField } from './recipient-field'
import { SendButton } from './send-button'
import { fieldsAreEmpty, useDraftEditor, type SaveStatus } from './use-draft-editor'
import { useMailBodyEditor } from './mail-body-editor-loader'

/** What the window around the composer needs from it when closing. */
export type ComposerController = {
  /** Writes pending edits; `ok` is false when something stays unsaved. */
  flush: () => Promise<{ ok: boolean }>
  /** True when recipients, subject and body are all empty. */
  isEmpty: () => boolean
  /** Whether the content differs from what the window opened with. */
  hasChanges: () => boolean
  /** Writes the content the window opened with back to the draft. */
  restoreBaseline: () => Promise<void>
  /** Deletes the draft (not while the send queue holds it). */
  discard: () => Promise<void>
  /** Whether the send queue holds the draft, so it cannot be changed or deleted. */
  isReadOnly: () => boolean
}

type ComposerProps = {
  client: MailxClient
  draftId: string
  accounts: Account[]
  /** The subject as typed, for the window title. */
  onSubjectChange?: (subject: string) => void
  /** The draft was queued; the host closes its window. */
  onSent: () => void
  /** The draft was deleted on request; the host closes its window. */
  onDiscarded: () => void
  controllerRef?: Ref<ComposerController>
}

type ValidationKey = 'composer:validation.recipientsRequired' | 'composer:validation.invalidRecipients' | 'composer:validation.senderUnavailable' | 'composer:validation.credentials' | 'composer:validation.unsaved' | 'composer:validation.alreadyQueued'
type ComposerProblem = { key: ValidationKey } | { cause: unknown }

const STATUS_TEXT: Record<SaveStatus, string> = { idle: '', get saved() { return t('composer:save.saved') }, get saving() { return t('composer:save.saving') }, get failed() { return t('composer:save.failed') } }

/**
 * The writing surface: sender, recipients, subject, rich-text body and the bottom
 * bar. It knows nothing about windows, so the same component can later sit in the
 * main window's island (sending.md section 2). Closing and window titles belong to
 * the host, which reaches in through `controllerRef`.
 */
export function Composer({ client, draftId, accounts, onSubjectChange, onSent, onDiscarded, controllerRef }: ComposerProps) {
  useTranslation('composer')
  // Also load on mount for other hosts, independently of the draft's response.
  const { Component: MailBodyEditor, failed: editorLoadFailed, retry: retryEditor } = useMailBodyEditor()
  const editor = useDraftEditor(client, draftId, (notice) => {
    if (notice.kind === 'conflict') toast.info(t('composer:notice.conflict'), { get description() { return t('composer:notice.conflictDescription') } })
    else toast.info(t('composer:notice.locked'))
  })
  const { fields } = editor
  const [showCc, setShowCc] = useState(false)
  const [showBcc, setShowBcc] = useState(false)
  const [error, setError] = useState<ComposerProblem | null>(null)
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  const [busy, setBusy] = useState(false)

  useImperativeHandle(controllerRef, () => ({
    flush: () => editor.flush(),
    isEmpty: () => editor.isEmpty(),
    hasChanges: () => editor.changed(),
    restoreBaseline: () => editor.restoreBaseline(),
    discard: () => editor.remove(),
    isReadOnly: () => editor.readOnly,
  }), [editor])

  useEffect(() => {
    if (fields) onSubjectChange?.(fields.subject)
  }, [fields, onSubjectChange])

  // A saved draft with Cc or Bcc recipients opens with those rows visible.
  const ccVisible = showCc || (fields?.cc.length ?? 0) > 0
  const bccVisible = showBcc || (fields?.bcc.length ?? 0) > 0

  const account = useMemo(() => accounts.find((row) => row.id === fields?.accountId) ?? null, [accounts, fields?.accountId])
  const accountItems = useMemo(() => Object.fromEntries(accounts.map((row) => [row.id, accountLabel(row)])), [accounts])

  if (editor.gone) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyMedia variant="icon"><HugeiconsIcon icon={MailOpen01Icon} /></EmptyMedia>
          <EmptyTitle>{t('composer:draft.gone')}</EmptyTitle>
          <EmptyDescription>{t('composer:draft.goneWindowDescription')}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }
  if (editor.problem) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyTitle>{t('composer:draft.openFailed')}</EmptyTitle>
          <EmptyDescription>{errorText(editor.problem)}</EmptyDescription>
        </EmptyHeader>
        <Button variant="outline" onClick={() => void editor.reload()}>{t('composer:action.retry')}</Button>
      </Empty>
    )
  }
  if (!fields) {
    return <div role="status" aria-label={t('composer:draft.opening')} className="flex flex-1 items-center justify-center"><Spinner className="size-5" /></div>
  }

  const readOnly = editor.readOnly
  const hasInvalid = [...fields.to, ...fields.cc, ...fields.bcc].some((item) => !item.valid)
  const empty = fieldsAreEmpty(fields)

  // First press of the send button: check, save, and hand back the saved revision.
  const prepare = async (): Promise<number | null> => {
    setError(null)
    const latest = editor.latest()
    if (!latest) return null
    const all = [...latest.to, ...latest.cc, ...latest.bcc]
    if (all.length === 0) {
      setError({ key: 'composer:validation.recipientsRequired' })
      return null
    }
    if (all.some((item) => !item.valid)) {
      setError({ key: 'composer:validation.invalidRecipients' })
      return null
    }
    const sender = accounts.find((row) => row.id === latest.accountId)
    if (!sender) {
      setError({ key: 'composer:validation.senderUnavailable' })
      return null
    }
    if (sender.authentication.smtp.trust !== 'trusted' || !sender.authentication.smtp.passwordConfigured) {
      setError({ key: 'composer:validation.credentials' })
      return null
    }
    const { ok } = await editor.flush()
    if (!ok) {
      setError({ key: 'composer:validation.unsaved' })
      return null
    }
    return editor.revision()
  }

  // Second press: queue at the revision that was saved when the button asked.
  const send = async (expectedRevision: number) => {
    setBusy(true)
    try {
      await client.queueSend({ draftId, expectedRevision, clientMutationId: crypto.randomUUID() })
      onSent()
    } catch (cause) {
      const code = clientFailureCode(cause)
      if (code === 'conflict') {
        toast.info(t('composer:notice.conflictBeforeSend'))
        await editor.reload()
      } else if (code === 'draftLocked') {
        editor.markLocked()
        setError({ key: 'composer:validation.alreadyQueued' })
      } else setError({ cause })
    } finally {
      setBusy(false)
    }
  }

  const discard = async () => {
    setConfirmDiscard(false)
    setBusy(true)
    try {
      await editor.remove()
      onDiscarded()
    } catch (cause) {
      setError({ cause })
    } finally {
      setBusy(false)
    }
  }

  const failedSave = editor.status === 'failed'
  const problemText = error ? ('key' in error ? t(error.key) : errorText(error.cause)) : null
  return (
    <section aria-label={t('composer:field.compose')} data-slot="composer" className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-col gap-0.5 px-3 pt-3">
        {readOnly && (
          <Alert className="mb-2" role="status">
            <AlertDescription>{t('composer:draft.lockedDescription')}</AlertDescription>
          </Alert>
        )}
        <div className="flex items-center gap-2 rounded-lg px-2 py-1">
          <span id="composer-from-label" className="w-14 shrink-0 text-xs text-muted-foreground">{t('composer:field.from')}</span>
          <Select
            value={fields.accountId}
            items={accountItems}
            disabled={readOnly || accounts.length === 0}
            onValueChange={(value: string | null) => value && editor.update({ accountId: value })}
          >
            <SelectTrigger
              aria-labelledby="composer-from-label"
              className="h-7 min-w-0 max-w-full border-transparent bg-transparent text-sm shadow-none hover:bg-muted/50 dark:bg-transparent dark:hover:bg-muted/50"
            >
              <SelectValue>{() => (account ? accountLabel(account) : t('composer:field.chooseSender'))}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {accounts.map((row) => (
                <SelectItem key={row.id} value={row.id}>
                  <span className="flex min-w-0 flex-col">
                    <span className="truncate">{accountLabel(row)}</span>
                    <span className="truncate text-[0.7rem] text-muted-foreground">{row.displayName}</span>
                  </span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <RecipientField
          label={t('composer:field.to')}
          value={fields.to}
          disabled={readOnly}
          autoFocus={fields.to.length === 0 && !fields.subject}
          onChange={(to) => editor.update({ to })}
          trailing={(
            <>
              {!ccVisible && <Button type="button" variant="ghost" size="xs" disabled={readOnly} onClick={() => setShowCc(true)}>{t('composer:field.cc')}</Button>}
              {!bccVisible && <Button type="button" variant="ghost" size="xs" disabled={readOnly} onClick={() => setShowBcc(true)}>{t('composer:field.bcc')}</Button>}
            </>
          )}
        />
        {ccVisible && <RecipientField label={t('composer:field.cc')} value={fields.cc} disabled={readOnly} onChange={(cc) => editor.update({ cc })} />}
        {bccVisible && <RecipientField label={t('composer:field.bcc')} value={fields.bcc} disabled={readOnly} onChange={(bcc) => editor.update({ bcc })} />}
        <div className="flex items-center gap-2 rounded-lg px-2 py-1 transition-colors focus-within:bg-muted/50 hover:bg-muted/30">
          <label htmlFor="composer-subject" className="w-14 shrink-0 text-xs text-muted-foreground">{t('composer:field.subject')}</label>
          <input
            id="composer-subject"
            value={fields.subject}
            disabled={readOnly}
            autoComplete="off"
            maxLength={998}
            onChange={(event) => editor.update({ subject: event.target.value })}
            className="h-6 min-w-0 flex-1 bg-transparent text-sm font-medium outline-none disabled:cursor-not-allowed"
          />
        </div>
      </div>
      {MailBodyEditor ? (
        <MailBodyEditor value={fields.bodyHtml} readOnly={readOnly} onChange={(bodyHtml) => editor.update({ bodyHtml })} />
      ) : editorLoadFailed ? (
        <div role="alert" className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2">
          <p>{t('composer:editor.loadFailed')}</p>
          <Button type="button" variant="outline" onClick={retryEditor}>{t('composer:editor.reload')}</Button>
        </div>
      ) : (
        <div role="status" className="flex min-h-0 flex-1 items-center justify-center">{t('composer:editor.opening')}</div>
      )}
      <div className="flex shrink-0 flex-wrap items-center gap-2 px-4 py-3">
        <div className="flex min-w-0 flex-1 items-center gap-2 text-xs">
          {/* The width and height are reserved so the status coming and going never moves the buttons. */}
          <span
            role="status"
            aria-live="polite"
            data-status={readOnly ? 'locked' : editor.status}
            className={cn('inline-flex h-6 min-w-20 shrink-0 items-center', failedSave && !readOnly ? 'text-destructive' : 'text-muted-foreground')}
          >
            <span key={readOnly ? 'locked' : editor.status} className="animate-in duration-150 fade-in-0 motion-reduce:animate-none">
              {readOnly ? t('composer:draft.queued') : STATUS_TEXT[editor.status]}
            </span>
          </span>
          {failedSave && !readOnly && <Button type="button" variant="ghost" size="xs" onClick={editor.retry}>{t('composer:action.retry')}</Button>}
          {(error || hasInvalid) && (
            <span role="alert" className="flex min-w-0 items-center gap-1 text-destructive">
              <HugeiconsIcon icon={AlertCircleIcon} className="size-3.5 shrink-0" />
              <span className="truncate" title={problemText ?? undefined}>{problemText ?? t('composer:validation.invalidAddress')}</span>
            </span>
          )}
        </div>
        <Button type="button" variant="ghost" className="h-8 px-3 text-sm" disabled={readOnly || busy}
          onClick={() => (empty ? void discard() : setConfirmDiscard(true))}>
          {t('composer:action.discard')}</Button>
        <SendButton
          subjectEmpty={!fields.subject.trim()}
          disabled={readOnly || busy}
          resetToken={editor.changeToken}
          onPrepare={prepare}
          onConfirm={send}
        />
      </div>
      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('composer:draft.discardTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t('composer:draft.deleteDescription')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('composer:action.continueEditing')}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => void discard()}>{t('composer:action.discard')}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}

function accountLabel(account: Account) {
  return account.senderName ? `${account.senderName} <${account.emailAddress}>` : account.emailAddress
}
