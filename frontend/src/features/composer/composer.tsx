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
import { clientFailure, clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { RecipientField } from './recipient-field'
import { SendButton } from './send-button'
import { useDraftEditor, type SaveStatus } from './use-draft-editor'

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

const STATUS_TEXT: Record<SaveStatus, string> = { idle: '', saved: '已保存', saving: '正在保存…', failed: '保存失败' }

/**
 * The writing surface: sender, recipients, subject, plain-text body and the bottom
 * bar. It knows nothing about windows, so the same component can later sit in the
 * main window's island (sending.md section 2). Closing and window titles belong to
 * the host, which reaches in through `controllerRef`.
 */
export function Composer({ client, draftId, accounts, onSubjectChange, onSent, onDiscarded, controllerRef }: ComposerProps) {
  const editor = useDraftEditor(client, draftId, (notice) => {
    if (notice.kind === 'conflict') toast.info('草稿已在别处修改，已加载最新版本', { description: '你刚才未保存的改动没有被应用。' })
    else toast.info('这封邮件已进入发送队列，不能再编辑')
  })
  const { fields } = editor
  const [showCc, setShowCc] = useState(false)
  const [showBcc, setShowBcc] = useState(false)
  const [error, setError] = useState<string | null>(null)
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
          <EmptyTitle>这份草稿已不存在</EmptyTitle>
          <EmptyDescription>它可能已被发送或删除。关闭此窗口即可。</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }
  if (editor.problem) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyTitle>无法打开草稿</EmptyTitle>
          <EmptyDescription>{editor.problem.message}</EmptyDescription>
        </EmptyHeader>
        <Button variant="outline" onClick={() => void editor.reload()}>重试</Button>
      </Empty>
    )
  }
  if (!fields) {
    return <div role="status" aria-label="正在打开草稿" className="flex flex-1 items-center justify-center"><Spinner className="size-5" /></div>
  }

  const readOnly = editor.readOnly
  const recipientCount = fields.to.length + fields.cc.length + fields.bcc.length
  const hasInvalid = [...fields.to, ...fields.cc, ...fields.bcc].some((item) => !item.valid)
  const empty = recipientCount === 0 && !fields.subject.trim() && !fields.bodyText.trim()

  // First press of the send button: check, save, and hand back the saved revision.
  const prepare = async (): Promise<number | null> => {
    setError(null)
    const latest = editor.latest()
    if (!latest) return null
    const all = [...latest.to, ...latest.cc, ...latest.bcc]
    if (all.length === 0) {
      setError('请至少填写一个收件人。')
      return null
    }
    if (all.some((item) => !item.valid)) {
      setError('有收件人地址无效，请修改或移除标红的地址。')
      return null
    }
    const sender = accounts.find((row) => row.id === latest.accountId)
    if (!sender) {
      setError('发件账号不可用，请重新选择。')
      return null
    }
    if (sender.authentication.smtp.trust !== 'trusted' || !sender.authentication.smtp.passwordConfigured) {
      setError('该账号的发信凭证需要重新确认，请先在主窗口的账号设置中处理。')
      return null
    }
    const { ok } = await editor.flush()
    if (!ok) {
      setError('草稿尚未保存成功，暂时不能发送。')
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
        toast.info('草稿已在别处修改，已加载最新版本，请确认后再发送')
        await editor.reload()
      } else if (code === 'draftLocked') {
        editor.markLocked()
        setError('这封邮件已经在发送队列中。')
      } else setError(clientFailure(cause).message)
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
      setError(clientFailure(cause).message)
    } finally {
      setBusy(false)
    }
  }

  const failedSave = editor.status === 'failed'
  return (
    <section aria-label="写邮件" data-slot="composer" className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-col gap-0.5 px-3 pt-3">
        {readOnly && (
          <Alert className="mb-2" role="status">
            <AlertDescription>这封邮件已进入发送队列，不能再修改。可在主窗口的发件箱里撤销或查看进度。</AlertDescription>
          </Alert>
        )}
        <div className="flex items-center gap-2 rounded-lg px-2 py-1">
          <span id="composer-from-label" className="w-14 shrink-0 text-xs text-muted-foreground">发件人</span>
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
              <SelectValue>{() => (account ? accountLabel(account) : '选择发件账号')}</SelectValue>
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
          label="收件人"
          value={fields.to}
          disabled={readOnly}
          autoFocus={fields.to.length === 0 && !fields.subject}
          onChange={(to) => editor.update({ to })}
          trailing={(
            <>
              {!ccVisible && <Button type="button" variant="ghost" size="xs" disabled={readOnly} onClick={() => setShowCc(true)}>抄送</Button>}
              {!bccVisible && <Button type="button" variant="ghost" size="xs" disabled={readOnly} onClick={() => setShowBcc(true)}>密送</Button>}
            </>
          )}
        />
        {ccVisible && <RecipientField label="抄送" value={fields.cc} disabled={readOnly} onChange={(cc) => editor.update({ cc })} />}
        {bccVisible && <RecipientField label="密送" value={fields.bcc} disabled={readOnly} onChange={(bcc) => editor.update({ bcc })} />}
        <div className="flex items-center gap-2 rounded-lg px-2 py-1 transition-colors focus-within:bg-muted/50 hover:bg-muted/30">
          <label htmlFor="composer-subject" className="w-14 shrink-0 text-xs text-muted-foreground">主题</label>
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
      <textarea
        aria-label="正文"
        value={fields.bodyText}
        disabled={readOnly}
        spellCheck
        onChange={(event) => editor.update({ bodyText: event.target.value })}
        className="mx-3 mt-1 min-h-0 flex-1 resize-none rounded-lg bg-transparent px-2 py-2 text-sm leading-relaxed outline-none disabled:cursor-not-allowed"
      />
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
              {readOnly ? '已在发送队列中' : STATUS_TEXT[editor.status]}
            </span>
          </span>
          {failedSave && !readOnly && <Button type="button" variant="ghost" size="xs" onClick={editor.retry}>重试</Button>}
          {(error || hasInvalid) && (
            <span role="alert" className="flex min-w-0 items-center gap-1 text-destructive">
              <HugeiconsIcon icon={AlertCircleIcon} className="size-3.5 shrink-0" />
              <span className="truncate" title={error ?? undefined}>{error ?? '有收件人地址无效'}</span>
            </span>
          )}
        </div>
        <Button type="button" variant="ghost" className="h-8 px-3 text-sm" disabled={readOnly || busy}
          onClick={() => (empty ? void discard() : setConfirmDiscard(true))}>
          丢弃
        </Button>
        <SendButton
          subjectEmpty={!fields.subject.trim()}
          disabled={readOnly || busy}
          resetToken={fields}
          onPrepare={prepare}
          onConfirm={send}
        />
      </div>
      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>丢弃这封草稿？</AlertDialogTitle>
            <AlertDialogDescription>草稿会从这台电脑上删除，无法恢复。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => void discard()}>丢弃</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}

function accountLabel(account: Account) {
  return account.senderName ? `${account.senderName} <${account.emailAddress}>` : account.emailAddress
}
