import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { Account, ComposeOpenMode, DraftKind, OutboxChanged } from '@/generated/accounts'
import { toast } from '@/components/ui/toast'
import { clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { composeKeys } from '@/lib/query-client'
import { reasonText, visibleOutbox } from './outbox-text'
import type { InlineComposer } from './use-inline-composer'

export function useDrafts(client: MailxClient) {
  return useQuery({ queryKey: composeKeys.drafts(), queryFn: () => client.listDrafts() })
}

/** Unfinished sends only (queued, sending, failed, unsettled unknown), minus failures already re-sent. */
export function useOutbox(client: MailxClient) {
  return useQuery({
    queryKey: composeKeys.outbox(),
    queryFn: async () => visibleOutbox(await client.listOutbox()),
  })
}

/** How one open was asked for. `flip`: Shift-click, so this once the other place than the setting. */
export type OpenIntent = { flip?: boolean }

/**
 * The intent of a click: Shift with the mouse flips the open mode (sending.md
 * section 17.7). Keyboard activation (`detail` 0) and shortcuts follow the setting.
 */
export function openIntent(event?: { shiftKey?: boolean; detail?: number } | null): OpenIntent | undefined {
  return event?.shiftKey && (event.detail ?? 0) > 0 ? { flip: true } : undefined
}

/**
 * Creating drafts and opening them where the setting says (main window only):
 * a composer window, or the inline composer in the reader (sending.md section 17.7).
 * `bodyNotCached` is the core asking for the original's body first: fetch it with the
 * existing body download, then try once more.
 */
export function useComposeActions(
  client: MailxClient,
  accounts: Account[],
  contextAccountId: string | null,
  { mode, inline }: { mode: ComposeOpenMode; inline: InlineComposer },
) {
  const queryClient = useQueryClient()
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const place = useCallback((intent?: OpenIntent): ComposeOpenMode => intent?.flip ? (mode === 'inline' ? 'window' : 'inline') : mode, [mode])

  const exclusive = useCallback(async (where: ComposeOpenMode, step: () => Promise<void>) => {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(true)
    try {
      await step()
    } catch (cause) {
      toast.error(where === 'inline' ? t('mail:inlineComposeOpenFailed') : t('composer:toast.openFailed'), { description: errorText(cause) })
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }, [])

  // `fresh`: the draft is created for this composer, so closing it unsaved deletes it.
  // Inline, the current draft is saved first; a failed save keeps it and opens nothing.
  const run = useCallback((create: () => Promise<string>, intent?: OpenIntent) => {
    const where = place(intent)
    return exclusive(where, async () => {
      if (where === 'inline') {
        if (!(await inline.leave())) return
        inline.show(await create(), true)
      } else {
        await client.openComposer(await create(), true)
      }
      void queryClient.invalidateQueries({ queryKey: composeKeys.drafts() })
    })
  }, [client, exclusive, inline, place, queryClient])

  const compose = useCallback((intent?: OpenIntent) => run(async () => {
    const known = contextAccountId && accounts.some((account) => account.id === contextAccountId) ? contextAccountId : null
    const draft = await client.createDraft({
      accountId: known, to: [], cc: [], bcc: [], subject: '', bodyHtml: '', clientMutationId: crypto.randomUUID(),
    })
    return draft.id
  }, intent), [accounts, client, contextAccountId, run])

  const respond = useCallback((emailId: string, kind: Exclude<DraftKind, 'new'>, intent?: OpenIntent) => run(async () => {
    const request = { emailId, kind, clientMutationId: crypto.randomUUID() }
    try {
      return (await client.createDraftFrom(request)).id
    } catch (cause) {
      if (clientFailureCode(cause) !== 'bodyNotCached') throw cause
      const fetched = await client.fetchBody(emailId)
      if (fetched.summary.bodyState !== 'ready') throw cause
      return (await client.createDraftFrom(request)).id
    }
  }, intent), [client, run])

  // One draft is edited in one place: a draft open inline moves to its window, and a
  // draft that has a window is brought forward there instead of opening inline.
  const openDraft = useCallback((draftId: string, intent?: OpenIntent) => {
    const where = place(intent)
    return exclusive(where, async () => {
      if (where === 'window') {
        if (inline.draftId() === draftId) await inline.popOut()
        else await client.openComposer(draftId, false)
        return
      }
      if (inline.draftId() === draftId) {
        inline.reveal()
        return
      }
      if (await client.focusComposer(draftId)) return
      if (!(await inline.leave())) return
      inline.show(draftId, false)
    })
  }, [client, exclusive, inline, place])

  return { busy, compose, respond, openDraft }
}

/**
 * Keeps the drafts and outbox queries live and tells the user what happened to a
 * send (sending.md section 6). `onOpenOutbox` selects an item in the outbox view.
 */
export function useComposeEvents(
  client: MailxClient,
  onOpenOutbox: (outboxId: string) => void,
  /**
   * `onChange` sees every outbox event; `onReopen` opens a draft whose send was undone
   * (where the open-mode setting says; a composer window when absent).
   */
  opts: { onChange?: (change: OutboxChanged) => void; onReopen?: (draftId: string) => Promise<void> } = {},
) {
  const queryClient = useQueryClient()
  const open = useRef(onOpenOutbox)
  useEffect(() => { open.current = onOpenOutbox })
  const changed = useRef(opts.onChange)
  useEffect(() => { changed.current = opts.onChange })
  const reopen = useRef(opts.onReopen)
  useEffect(() => { reopen.current = opts.onReopen })
  const undoToasts = useRef(new Map<string, string>())

  useEffect(() => {
    let alive = true
    const forget = (outboxId: string) => {
      const id = undoToasts.current.get(outboxId)
      if (id) toast.close(id)
      undoToasts.current.delete(outboxId)
    }

    const offDrafts = client.onDrafts(() => void queryClient.invalidateQueries({ queryKey: composeKeys.drafts() }))
    const offOutbox = client.onOutbox((change: OutboxChanged) => {
      void queryClient.invalidateQueries({ queryKey: composeKeys.outbox() })
      void queryClient.invalidateQueries({ queryKey: composeKeys.drafts() })
      changed.current?.(change)
      if (change.state === 'queued' && change.attempts === 0) {
        // Just queued: say when it goes out, with a way back.
        void client.getOutbox(change.outboxId).then((item) => {
          if (!alive || item.state !== 'queued') return
          // No undo window (the default) means nothing to announce beyond "正在发送".
          if (new Date(item.notBefore).getTime() - new Date(item.firstQueuedAt).getTime() < 1000) return
          const seconds = Math.ceil((new Date(item.notBefore).getTime() - Date.now()) / 1000)
          if (seconds <= 0) return
          undoToasts.current.set(item.id, toast.info(t('composer:toast.sendAfter', { count: seconds }), {
            timeout: seconds * 1000,
            action: { label: t('composer:action.undo'), onClick: () => void undo(item.id, item.draftId) },
          }))
        }, () => undefined)
        return
      }
      forget(change.outboxId)
      if (change.state === 'sending') toast.info(t('composer:send.sending'), { timeout: 2000 })
      else if (change.state === 'accepted') toast.success(t('composer:toast.sent'))
      else if (change.state === 'failed') {
        toast.error(t('composer:toast.sendFailed', { reason: reasonText(change.errorReason) }), { action: { label: t('composer:action.view'), onClick: () => open.current(change.outboxId) } })
      } else if (change.state === 'unknown') {
        toast.error(t('composer:toast.unknown'), { action: { label: t('composer:action.view'), onClick: () => open.current(change.outboxId) } })
      }
    })

    async function undo(outboxId: string, draftId: string | null) {
      try {
        await client.cancelOutbox(outboxId)
        if (draftId) await (reopen.current ? reopen.current(draftId) : client.openComposer(draftId))
      } catch (cause) {
        if (clientFailureCode(cause) === 'alreadySending') toast.error(t('composer:detail.alreadySending'))
        else toast.error(t('composer:toast.undoFailed'), { description: errorText(cause) })
      }
    }

    const toasts = undoToasts.current
    return () => {
      alive = false
      offDrafts()
      offOutbox()
      toasts.forEach((id) => toast.close(id))
      toasts.clear()
    }
  }, [client, queryClient])
}
