import { errorText } from '@/i18n/errors'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode, type Ref } from 'react'
import { DragDropProvider } from '@dnd-kit/react'
import { Accessibility, Feedback, PointerActivationConstraints, PointerSensor } from '@dnd-kit/dom'
import { HugeiconsIcon } from '@hugeicons/react'
import { AlertCircleIcon, Attachment01Icon, MailOpen01Icon } from '@hugeicons/core-free-icons'
import type { Account, AttachmentPolicy } from '@/generated/accounts'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { IconButton } from '@/components/icon-button'
import { MOD_KEY } from '@/features/mail/shortcuts'
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
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { Spinner } from '@/components/ui/spinner'
import { toast } from '@/components/ui/toast'
import { clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { mergeRecipients, type Recipient } from './address-parse'
import { RecipientField, type RecipientDragData, type RecipientDropData, type RecipientFieldHandle, type RecipientRow } from './recipient-field'
import { RowToggle } from './row-toggle'
import { SendButton, type SendButtonHandle } from './send-button'
import { draftMentionsAttachment, foldsReplyQuote } from './reply-quote'
import { AttachmentBar } from './attachment-bar'
import type { MailBodyEditorHandle } from './mail-body-editor'
import { DraftUnsavedError, fieldsAreEmpty, useDraftEditor, type SaveStatus } from './use-draft-editor'
import { useMailBodyEditor } from './mail-body-editor-loader'
import { useBodyFocus } from './use-body-focus'
import { SenderPicker } from './sender-picker'

/** What the window around the composer needs from it when closing. */
export type ComposerController = {
  /** Writes pending edits; `ok` is false when something stays unsaved. */
  flush: () => Promise<{ ok: boolean }>
  /** True when recipients, subject and body are all empty. */
  isEmpty: () => boolean
  /**
   * Whether the draft's content has been loaded. Before that `isEmpty` is false
   * (nothing is known), so a host leaving early can tell "not loaded" from "has content".
   */
  isLoaded: () => boolean
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
  /**
   * Host controls for the top-right corner of the header (e.g. "open in a new
   * window" and "close" when the composer sits in the main window). Shown both
   * expanded and collapsed, right of the MCP / AI note.
   */
  headerActions?: ReactNode
  /** Opens the assistant's island about this draft, from writing help ("继续问"); the inline composer only. */
  onAskAi?: () => void
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
export function Composer({ client, draftId, accounts, onSubjectChange, onSent, onDiscarded, controllerRef, headerActions, onAskAi }: ComposerProps) {
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
    isLoaded: () => editor.latest() !== null,
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
  const toRef = useRef<RecipientFieldHandle>(null)
  const ccRef = useRef<RecipientFieldHandle>(null)
  const bccRef = useRef<RecipientFieldHandle>(null)
  const rowRefs = useMemo(() => ({ to: toRef, cc: ccRef, bcc: bccRef }), [])
  const pendingFocus = useRef<RecipientRow | null>(null)
  const body = useBodyFocus()
  const leaveBody = body.leave
  const reducedMotion = usePrefersReducedMotion()
  // A reply opens with the body focused and so already folded: that first fold must not animate.
  const [userActed, setUserActed] = useState(false)
  const sendRef = useRef<SendButtonHandle>(null)
  // The reminder belongs to the content it was raised for: any edit or added file withdraws it.
  const [reminderAt, setReminderAt] = useState<string | null>(null)

  const suggest = useCallback((query: string, limit?: number) => client.suggestContacts(query, limit), [client])
  // Addresses in any row are not suggested again, in any row.
  const taken = useMemo(() => new Set([...(fields?.to ?? []), ...(fields?.cc ?? []), ...(fields?.bcc ?? [])].map((item) => item.address.toLowerCase())), [fields?.to, fields?.cc, fields?.bcc])
  const account = useMemo(() => accounts.find((row) => row.id === fields?.accountId) ?? null, [accounts, fields?.accountId])

  // Focus a row once it is rendered and no longer inert (after opening it or expanding the header).
  const focusRow = useCallback((row: RecipientRow) => {
    if (row === 'cc') setShowCc(true)
    if (row === 'bcc') setShowBcc(true)
    leaveBody()
    pendingFocus.current = row
  }, [leaveBody])
  useLayoutEffect(() => {
    const row = pendingFocus.current
    if (!row) return
    pendingFocus.current = null
    rowRefs[row].current?.focus()
  })

  // An emptied Cc or Bcc row folds away once focus leaves it.
  const rowFocus = useCallback((row: RecipientRow, focused: boolean) => {
    if (focused) {
      leaveBody()
      if (row === 'cc') setShowCc(true)
      if (row === 'bcc') setShowBcc(true)
      return
    }
    const value = editor.latest()?.[row] ?? []
    if (value.length > 0) return
    if (row === 'cc') setShowCc(false)
    if (row === 'bcc') setShowBcc(false)
  }, [leaveBody, editor])

  // Dragging a chip or its menu: the address leaves one row for another, once.
  const moveRecipient = useCallback((from: RecipientRow, index: number, to: RecipientRow) => {
    const latest = editor.latest()
    const item: Recipient | undefined = latest?.[from][index]
    if (!latest || !item || from === to) return
    const rest = latest[from].filter((_, position) => position !== index)
    editor.update({ [from]: rest, [to]: mergeRecipients(latest[to], [item]) })
    if (to === 'cc') setShowCc(true)
    if (to === 'bcc') setShowBcc(true)
    if (rest.length === 0 && !rowRefs[from].current?.hasFocus()) {
      if (from === 'cc') setShowCc(false)
      if (from === 'bcc') setShowBcc(false)
    }
  }, [editor, rowRefs])

  // Cmd/Ctrl+Alt+C and +B, matched by physical key: on macOS Option changes `key` (ç, ∫).
  const onShortcut = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing || !(event.metaKey || event.ctrlKey) || !event.altKey || event.shiftKey) return
    // Ctrl+Alt is AltGr on many Windows layouts, where it types a character.
    if (event.nativeEvent.getModifierState?.('AltGraph')) return
    const row = event.code === 'KeyC' ? 'cc' : event.code === 'KeyB' ? 'bcc' : null
    if (!row || editor.readOnly || !fields) return
    event.preventDefault()
    event.stopPropagation()
    focusRow(row)
  }
  const accountId = fields?.accountId
  const [policy, setPolicy] = useState<AttachmentPolicy | null>(null)
  useEffect(() => {
    if (!accountId) return
    let current = true
    client.attachmentPolicy(accountId).then((next) => current && setPolicy(next), () => current && setPolicy(null))
    return () => { current = false }
  }, [client, accountId])

  // Every attachment command reports its failure in the bottom bar, like sending does.
  const { addAttachments, addDroppedAttachments, addInlineImages, addInlineImageBytes, removeAttachment } = editor
  const runAttachment = useCallback(async (command: () => Promise<unknown>) => {
    setError(null)
    try {
      await command()
    } catch (cause) {
      setError(cause instanceof DraftUnsavedError ? { key: 'composer:validation.unsaved' } : { cause })
    }
  }, [])
  // The host tells the window when the OS dropped files on it; it keeps the paths itself.
  useEffect(() => client.onFilesDropped(() => void runAttachment(addDroppedAttachments)), [client, runAttachment, addDroppedAttachments])
  const bodyRef = useRef<MailBodyEditorHandle | null>(null)
  const [missingImages, setMissingImages] = useState(0)
  // Each stored image is inserted where the cursor is once the host has given it a Content-ID.
  const insertPicked = useCallback(
    () => void runAttachment(async () => { bodyRef.current?.insertInlineImages(await addInlineImages()) }),
    [addInlineImages, runAttachment],
  )
  // One image at a time: every add flushes the body first, so the previous node is saved (and referenced) by then.
  const pasteImages = useCallback(
    (files: File[]) => void runAttachment(async () => {
      for (const file of files) bodyRef.current?.insertInlineImages(await addInlineImageBytes(file))
    }),
    [addInlineImageBytes, runAttachment],
  )
  const loadPreview = useCallback((attachmentId: string) => client.inlineImagePreview(draftId, attachmentId), [client, draftId])

  const regularAttachments = editor.attachments.filter((item) => !item.inline).length
  const reminderKey = `${editor.changeToken}:${regularAttachments}`
  // The host's corner controls sit over the first header line; that line keeps clear of them.
  const [actionsWidth, setActionsWidth] = useState(0)
  const measureActions = useCallback((element: HTMLDivElement | null) => {
    if (!element) return
    const measure = () => setActionsWidth(element.offsetWidth)
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

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
  const empty = fieldsAreEmpty(fields) && editor.attachments.length === 0

  // First press of the send button: check, save, and hand back the saved revision.
  // "仍要发送" runs it again without the forgotten-attachment check.
  const prepare = async ({ skipAttachmentCheck = false } = {}): Promise<number | null> => {
    if (!editor.isAvailable()) return null
    setError(null)
    setReminderAt(null)
    const latest = editor.latest()
    if (!latest) return null
    setMissingImages(0)
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
    const authentication = sender.authentication
    const usable = authentication.kind === 'ewsNtlm'
      ? authentication.credential.trust === 'trusted' && authentication.credential.passwordConfigured
      : authentication.smtp.trust === 'trusted' && (authentication.kind === 'password' ? authentication.smtp.passwordConfigured : authentication.state === 'connected' && authentication.smtp.credentialConfigured)
    if (!usable) {
      setError({ key: 'composer:validation.credentials' })
      return null
    }
    if (!skipAttachmentCheck && regularAttachments === 0
      && draftMentionsAttachment(latest, editor.kind)) {
      setReminderAt(reminderKey)
      return null
    }
    const { ok } = await editor.flush()
    if (!editor.isAvailable()) return null
    if (!ok) {
      setError({ key: 'composer:validation.unsaved' })
      return null
    }
    // Pictures whose attachment is gone are left out of the sent mail; say so, but let the send go on.
    const present = new Set(editor.attachments.filter((item) => item.inline).map((item) => item.contentId))
    const referenced = new DOMParser().parseFromString(latest.bodyHtml, 'text/html').querySelectorAll('img[src^="cid:"]')
    setMissingImages([...referenced].filter((image) => !present.has((image.getAttribute('src') ?? '').slice('cid:'.length))).length)
    return editor.revision()
  }

  // Second press: queue at the revision that was saved when the button asked.
  const send = async (expectedRevision: number) => {
    if (!editor.isAvailable()) return
    setBusy(true)
    try {
      await client.queueSend({ draftId, expectedRevision, clientMutationId: crypto.randomUUID() })
      if (!editor.isAvailable()) return
      onSent()
    } catch (cause) {
      if (!editor.isAvailable()) return
      const code = clientFailureCode(cause)
      if (code === 'conflict') {
        toast.info(t('composer:notice.conflictBeforeSend'))
        await editor.reload()
      } else if (code === 'draftLocked') {
        editor.markLocked()
        setError({ key: 'composer:validation.alreadyQueued' })
      } else setError({ cause })
    } finally {
      if (editor.isAvailable()) setBusy(false)
    }
  }

  const discard = async () => {
    if (!editor.isAvailable()) return
    setConfirmDiscard(false)
    setBusy(true)
    try {
      await editor.remove()
      if (!editor.isAvailable()) return
      onDiscarded()
    } catch (cause) {
      if (!editor.isAvailable()) return
      setError({ cause })
    } finally {
      if (editor.isAvailable()) setBusy(false)
    }
  }

  const failedSave = editor.status === 'failed'
  const problemText = error ? ('key' in error ? t(error.key) : errorText(error.cause)) : null
  const recipients = [...fields.to, ...fields.cc, ...fields.bcc]
  const collapsed = body.active && recipients.length > 0 && !hasInvalid && !error
  const reminder = reminderAt === reminderKey && !error && !hasInvalid
  const summary = headerSummary(account?.displayName ?? fields.accountId, recipients)
  const grid = cn('grid', userActed && !reducedMotion && 'transition-[grid-template-rows] duration-150 ease-out')
  const acted = () => { if (!userActed) setUserActed(true) }
  const origin = editor.origin === 'mcp' ? t('composer:draft.createdViaMcp') : editor.origin === 'ai' ? t('composer:draft.createdViaAi') : null
  const recipientRow = (row: RecipientRow, label: string, extra: Partial<Parameters<typeof RecipientField>[0]> = {}) => (
    <RecipientField
      label={label}
      value={fields[row]}
      disabled={readOnly}
      row={row}
      fieldRef={rowRefs[row]}
      onChange={(next) => editor.update({ [row]: next })}
      onMove={(index, to) => moveRecipient(row, index, to)}
      onFocusChange={(focused) => rowFocus(row, focused)}
      suggest={suggest}
      exclude={taken}
      {...extra}
    />
  )
  return (
    <section aria-label={t('composer:field.compose')} data-slot="composer" className="flex min-h-0 flex-1 flex-col"
      onPointerDownCapture={acted} onKeyDownCapture={(event) => { acted(); onShortcut(event) }}>
      <div className="flex shrink-0 flex-col gap-0.5 px-3 pt-3">
        {readOnly && (
          <Alert className="mb-2" role="status">
            <AlertDescription>{t('composer:draft.lockedDescription')}</AlertDescription>
          </Alert>
        )}
        <div className="relative" style={headerActions ? { '--composer-actions': `${actionsWidth}px` } as React.CSSProperties : undefined}>
          {/* Sender and recipients fold into one summary line while the user writes the body (sending.md section 17.2). */}
          <div className={cn(grid, collapsed ? 'grid-rows-[0fr]' : 'grid-rows-[1fr]')} data-slot="composer-header-rows" data-collapsed={collapsed || undefined}>
            <div className="flex min-h-0 flex-col gap-0.5 overflow-hidden" inert={collapsed}>
              <DragDropProvider
                sensors={[PointerSensor.configure({ activationConstraints: dragActivation })]}
                plugins={(defaults) => defaults.filter((plugin) => plugin !== Accessibility).map((plugin) => plugin === Feedback ? Feedback.configure({ dropAnimation: null }) : plugin)}
                onDragEnd={(event) => {
                  const source = event.operation.source?.data as RecipientDragData | undefined
                  const target = event.operation.target?.data as RecipientDropData | undefined
                  if (!event.canceled && source && target) moveRecipient(source.row, source.index, target.row)
                }}
              >
                <div className="flex items-center gap-2 rounded-lg py-1 pl-2 pr-[calc(var(--composer-actions,0px)+0.5rem)]">
                  <span id="composer-from-label" className="w-14 shrink-0 text-xs leading-6 text-muted-foreground">{t('composer:field.from')}</span>
                  <div className="-ml-2 flex min-w-0 flex-1 items-center">
                    <SenderPicker accounts={accounts} value={fields.accountId} disabled={readOnly} labelId="composer-from-label"
                      onChange={(accountId) => editor.update({ accountId })} onFocus={leaveBody} />
                  </div>
                  {origin && <span className="shrink-0 truncate text-xs text-muted-foreground">{origin}</span>}
                </div>
                {recipientRow('to', t('composer:field.to'), {
                  autoFocus: fields.to.length === 0 && !fields.subject,
                  trailing: (!ccVisible || !bccVisible) && (
                    <>
                      {!ccVisible && <RowToggle row="cc" label={t('composer:field.cc')} tip={t('composer:field.ccShortcut', { mod: MOD_KEY })} disabled={readOnly} onOpen={() => focusRow('cc')} />}
                      {!bccVisible && <RowToggle row="bcc" label={t('composer:field.bcc')} tip={t('composer:field.bccShortcut', { mod: MOD_KEY })} disabled={readOnly} onOpen={() => focusRow('bcc')} />}
                    </>
                  ),
                })}
                {ccVisible && recipientRow('cc', t('composer:field.cc'))}
                {bccVisible && recipientRow('bcc', t('composer:field.bcc'))}
              </DragDropProvider>
            </div>
          </div>
          <div className={cn(grid, collapsed ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]')}>
            <div className="min-h-0 overflow-hidden" inert={!collapsed}>
              <button
                type="button"
                data-slot="composer-header-summary"
                aria-label={t('composer:header.expand', { summary })}
                onClick={() => focusRow('to')}
                className="flex w-full min-w-0 items-center gap-2 rounded-lg py-1 pl-2 pr-[calc(var(--composer-actions,0px)+0.5rem)] text-left outline-none transition-colors hover:bg-muted/30 focus-visible:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring/30 motion-reduce:transition-none"
              >
                <span aria-hidden="true" className="w-14 shrink-0 text-xs leading-6 text-muted-foreground">{t('composer:field.from')}</span>
                <span aria-hidden="true" className="flex min-w-0 flex-1 items-center gap-1.5 text-sm">
                  {account && <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-5 text-[10px]" />}
                  <span className="max-w-[45%] shrink-0 truncate">{account?.displayName}</span>
                  <span className="shrink-0 text-muted-foreground">→</span>
                  <span className="min-w-0 truncate">{summaryRecipients(recipients)}</span>
                </span>
                {origin && <span aria-hidden="true" className="shrink-0 truncate text-xs text-muted-foreground">{origin}</span>}
              </button>
            </div>
          </div>
          {headerActions && (
            <div ref={measureActions} data-slot="composer-header-actions" className="absolute top-0.5 right-0 flex items-center gap-0.5">{headerActions}</div>
          )}
        </div>
        <div className="flex items-center gap-2 rounded-lg px-2 py-1 transition-colors focus-within:bg-muted/50 hover:bg-muted/30 motion-reduce:transition-none">
          <label htmlFor="composer-subject" className="w-14 shrink-0 text-xs leading-6 text-muted-foreground">{t('composer:field.subject')}</label>
          <input
            id="composer-subject"
            value={fields.subject}
            disabled={readOnly}
            autoComplete="off"
            maxLength={998}
            onChange={(event) => editor.update({ subject: event.target.value })}
            className="h-6 min-w-0 flex-1 bg-transparent text-[0.9375rem] font-medium outline-none disabled:cursor-not-allowed"
          />
        </div>
      </div>
      <div className="contents" {...body.handlers}>
      {MailBodyEditor ? (
        <MailBodyEditor
          value={fields.bodyHtml}
          readOnly={readOnly}
          onChange={(bodyHtml) => editor.update({ bodyHtml })}
          editorRef={bodyRef}
          inlineImages={{ attachments: editor.attachments, loadPreview }}
          onInsertImage={insertPicked}
          onPasteImages={pasteImages}
          foldReplyQuote={foldsReplyQuote(editor.kind)}
          ai={{ client, draftId, kind: editor.kind, onAskAi }}
        />
      ) : editorLoadFailed ? (
        <div role="alert" className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2">
          <p>{t('composer:editor.loadFailed')}</p>
          <Button type="button" variant="outline" onClick={retryEditor}>{t('composer:editor.reload')}</Button>
        </div>
      ) : (
        <div role="status" className="flex min-h-0 flex-1 items-center justify-center">{t('composer:editor.opening')}</div>
      )}
      </div>
      <AttachmentBar
        attachments={editor.attachments}
        policy={policy}
        readOnly={readOnly}
        busy={editor.attaching}
        onRemove={(id) => void runAttachment(() => removeAttachment(id))}
      />
      {/* Its own row above the bar, so the whole sentence fits and the bar's buttons never move (sending.md section 17.6). */}
      {reminder && (
        <div role="alert" data-slot="attachment-reminder"
          className="mx-3 flex shrink-0 animate-in flex-wrap items-center gap-x-1 gap-y-0.5 rounded-lg bg-muted/50 py-1 pr-1 pl-3 text-xs duration-150 fade-in-0 motion-reduce:animate-none">
          <span className="mr-1">{t('composer:notice.attachmentMentioned')}</span>
          <span className="flex items-center gap-1">
            <Button type="button" variant="ghost" size="sm" className="hover:bg-muted" disabled={readOnly || busy || editor.attaching}
              onClick={() => void runAttachment(addAttachments)}>{t('composer:notice.addAttachment')}</Button>
            <Button type="button" variant="ghost" size="sm" className="hover:bg-muted" disabled={readOnly || busy}
              onClick={() => sendRef.current?.prepare(() => prepare({ skipAttachmentCheck: true }))}>{t('composer:notice.sendAnyway')}</Button>
          </span>
        </div>
      )}
      <div className="flex shrink-0 flex-wrap items-center gap-2 px-4 py-3">
        <IconButton
          type="button"
          disabled={readOnly || busy || editor.attaching}
          label={t('composer:attachment.add')}
          onClick={() => void runAttachment(addAttachments)}
        >
          <HugeiconsIcon icon={Attachment01Icon} aria-hidden="true" />
        </IconButton>
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
          {missingImages > 0 && !error && !hasInvalid && (
            <span role="status" className="truncate text-muted-foreground">{t('composer:notice.missingImages', { count: missingImages })}</span>
          )}
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
          controlRef={sendRef}
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

// A click stays a click: chips start moving after a few pixels (touch after a short hold).
const dragActivation = (event: PointerEvent) => event.pointerType === 'touch'
  ? [new PointerActivationConstraints.Delay({ value: 250, tolerance: 5 })]
  : [new PointerActivationConstraints.Distance({ value: 4 })]

/** The first two recipients by name, then how many there are in all. */
function summaryRecipients(recipients: Recipient[]) {
  const names = recipients.slice(0, 2).map((item) => item.name || item.address).join(t('composer:header.separator'))
  return recipients.length > 2 ? t('composer:header.more', { names, total: recipients.length }) : names
}

function headerSummary(accountName: string, recipients: Recipient[]) {
  return `${accountName} → ${summaryRecipients(recipients)}`
}
