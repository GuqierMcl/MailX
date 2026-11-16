import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, Cancel01Icon, File01Icon, Loading03Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { toast } from '@/components/ui/toast'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import type { ToolUndo } from '@/generated/accounts'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { displayName, formatFullTime } from '@/features/mail/format'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { cn } from '@/lib/utils'
import { toolEmailIds } from './chat-model'
import { CHIP_LIMIT, UNDO_WINDOW_MS, toolLabel } from './tool-labels'
import { useChatEnv } from './chat-env'
import type { RunTool } from './use-assistant-run'

async function runUndo(client: MailxClient, undo: ToolUndo) {
  if (undo.type === 'archive') return client.undoArchive(undo.token)
  // Each flag goes back to the value it had, in as few calls as there are distinct values.
  const groups = new Map<string, { flag: 'read' | 'starred'; value: boolean; ids: string[] }>()
  for (const item of undo.restore) {
    for (const [flag, value] of [['read', item.isRead], ['starred', item.isStarred]] as const) {
      if (value === null) continue
      const key = `${flag}:${value}`
      const group = groups.get(key) ?? { flag, value, ids: [] }
      group.ids.push(item.emailId)
      groups.set(key, group)
    }
  }
  await Promise.all([...groups.values()].map((group) => client.setFlags(group.ids, group.flag, group.value)))
}

/** "撤销" for a finished organize step: for a minute, once; then it is gone. */
function UndoButton({ tool, undo }: { tool: RunTool; undo: ToolUndo }) {
  const { client, undone, markUndone } = useChatEnv()
  const [busy, setBusy] = useState(false)
  const [gone, setGone] = useState(false)
  const [started] = useState(() => tool.settledAt ?? Date.now())
  const [expired, setExpired] = useState(false)
  useEffect(() => {
    const timer = setTimeout(() => setExpired(true), Math.max(0, started + UNDO_WINDOW_MS - Date.now()))
    return () => clearTimeout(timer)
  }, [started])
  if (undone.has(tool.callId)) return <span className="text-muted-foreground">{t('ai:chat.undone')}</span>
  if (expired || gone) return null
  const run = () => {
    if (busy) return
    setBusy(true)
    runUndo(client, undo).then(() => markUndone(tool.callId), (error: unknown) => {
      setBusy(false)
      // The host no longer has it (the time ran out there, or something else touched those emails).
      if (error instanceof MailxClientError && error.failure.code === 'notFound') setGone(true)
      else toast.error(`${t('ai:chat.undoFailed')} ${errorText(error)}`)
    })
  }
  return (
    <>
      <span aria-hidden>·</span>
      <button type="button" onClick={run} disabled={busy} className="rounded-sm text-foreground/80 underline-offset-2 outline-none hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50">
        {t('ai:chat.undo')}
      </button>
    </>
  )
}

/** A small tag for one email an answer or step refers to; it opens the mail the way the page's own rows do. */
export function EmailChip({ emailId }: { emailId: string }) {
  const { client, onOpenEmail } = useChatEnv()
  const email = useQuery({
    queryKey: [...mailKeys.all, 'assistant-email', emailId],
    queryFn: () => client.getEmail(emailId),
    staleTime: 60_000,
    retry: false,
  })
  const summary = email.data?.summary
  const subject = summary ? summary.subject || t('mail:noSubject') : t('ai:chat.unknownEmail')
  const chip = (
    <button
      type="button"
      disabled={!summary}
      aria-label={summary ? `${t('ai:chat.openEmail')}: ${displayName(summary.from)} · ${subject}` : t('ai:chat.unknownEmail')}
      onClick={() => summary && onOpenEmail({ id: summary.id, accountId: summary.accountId })}
      className="flex max-w-64 items-center gap-1.5 rounded-full bg-muted/60 py-0.5 pr-2.5 pl-0.5 text-xs text-foreground outline-none hover:bg-foreground/8 focus-visible:ring-2 focus-visible:ring-ring disabled:text-muted-foreground disabled:hover:bg-muted/60"
    >
      {summary && <SenderAvatar sender={summary.from} accountId={summary.accountId} size="sm" className="size-4 text-[8px]" />}
      <span className="truncate">{summary ? `${displayName(summary.from)} · ${subject}` : subject}</span>
    </button>
  )
  if (!summary) return chip
  return (
    <Tooltip>
      <TooltipTrigger render={chip} />
      <TooltipContent>
        <span className="block max-w-72 wrap-anywhere">{displayName(summary.from)} · {formatFullTime(summary.receivedAt)}<br />{subject}</span>
      </TooltipContent>
    </Tooltip>
  )
}

/** What the assistant wrote or changed in a draft: subject, who it goes to when known, and a way to open it. */
function DraftCard({ draftId, subject }: { draftId: string; subject: string }) {
  const { client, onOpenDraft } = useChatEnv()
  const draft = useQuery({ queryKey: ['assistant', 'draft', draftId], queryFn: () => client.getDraft(draftId), staleTime: 5_000, retry: false })
  const recipients = draft.data ? draft.data.to.map((to) => displayName(to)).join('、') : ''
  return (
    <div data-slot="draft-card" className="flex items-center gap-3 rounded-xl bg-muted/50 py-2 pr-2 pl-3.5 text-sm">
      <HugeiconsIcon icon={File01Icon} aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{subject.trim() || t('mail:noSubject')}</span>
        <span className="block truncate text-xs text-muted-foreground">{draft.data ? recipients || t('ai:chat.noRecipients') : t('ai:chat.draftBy')}</span>
      </span>
      <Button type="button" variant="ghost" size="lg" onClick={() => onOpenDraft(draftId)}>{t('ai:chat.openDraft')}</Button>
    </div>
  )
}

/**
 * One step the assistant took, as a quiet line (ai-chat-ui.md 4.2). Searching and reading collapse to
 * "搜索了 12 封邮件" and can list the emails they touched; an organize step carries its undo; a
 * draft becomes a card. No raw tool names, arguments or output are shown.
 */
export function ToolRow({ tool }: { tool: RunTool }) {
  useTranslation()
  const [open, setOpen] = useState(false)
  const { summary } = tool
  if (tool.status === 'done' && summary.type === 'draft' && tool.name !== 'delete_draft') return <DraftCard draftId={summary.draftId} subject={summary.subject} />
  const ids = tool.status === 'done' ? toolEmailIds(tool) : []
  const undo = tool.status === 'done' && summary.type === 'changed' ? summary.undo : null
  const label = toolLabel(tool)
  const running = tool.status === 'running'
  const bad = tool.status === 'failed' || tool.status === 'denied'
  return (
    <div data-slot="tool-row" data-status={tool.status} className="flex flex-col gap-1.5 text-sm text-muted-foreground">
      <div className="flex min-h-6 flex-wrap items-center gap-x-1.5">
        <HugeiconsIcon
          icon={running ? Loading03Icon : bad ? Cancel01Icon : Tick02Icon}
          aria-hidden
          strokeWidth={2}
          className={cn('size-3.5 shrink-0', running && 'motion-safe:animate-spin', !running && 'opacity-60')}
        />
        {/* The words change in place with a short fade; the line keeps its height. */}
        <span key={tool.status} className="animate-now-text-in">{label}</span>
        {undo && <UndoButton tool={tool} undo={undo} />}
        {ids.length > 0 && (
          <button
            type="button"
            aria-expanded={open}
            aria-label={open ? t('ai:chat.hideDetails') : t('ai:chat.details')}
            onClick={() => setOpen((value) => !value)}
            className="flex size-5 items-center justify-center rounded-sm outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
          >
            <HugeiconsIcon icon={ArrowDown01Icon} aria-hidden strokeWidth={2} className={cn('size-3.5 transition-transform duration-150 motion-reduce:transition-none', open && 'rotate-180')} />
          </button>
        )}
      </div>
      {open && ids.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5 pl-5">
          {ids.slice(0, CHIP_LIMIT).map((id) => <EmailChip key={id} emailId={id} />)}
          {ids.length > CHIP_LIMIT && <span className="text-xs">{t('ai:chat.moreEmails', { count: ids.length - CHIP_LIMIT })}</span>}
        </div>
      )}
    </div>
  )
}
