import { useCallback, useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import type { EmailSummary, Folder, SelectedEmail } from '@/generated/accounts'
import { toast } from '@/components/ui/toast'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { SELECTION_LIMIT } from './use-list-selection'

/**
 * Whether deleting `email` expunges it for good instead of moving it to Trash:
 * it already is in Trash, or its account has none. The core makes the same
 * call and refuses a permanent delete the user did not confirm.
 */
export function deletesForGood(email: Pick<EmailSummary, 'accountId' | 'folderId'>, folders: Folder[]) {
  const source = folders.find((folder) => folder.id === email.folderId)
  if (source?.role === 'trash') return true
  return !folders.some((folder) => folder.accountId === email.accountId && folder.role === 'trash' && folder.selectable)
}

// The core's window is a few seconds; a skewed clock must neither close the
// toast at once nor overflow the browser's 32-bit timer (which also fires at once).
const MAX_UNDO_TOAST_MS = 60_000

function undoTimeout(undoUntil: string) {
  const left = Date.parse(undoUntil) - Date.now()
  return Number.isFinite(left) ? Math.min(Math.max(left, 0), MAX_UNDO_TOAST_MS) : 0
}

/**
 * Deletes server emails with a few seconds to take it back. The email leaves
 * every list at once; the core only touches the server once the undo window
 * of the returned deletion passes, and the toast stays up until then.
 * `onHidden` runs once an email is hidden, so the reader can let go of it.
 */
export function useDeleteEmail(client: MailxClient, folders: Folder[], onHidden: (email: { id: string }) => void) {
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState<Confirming | null>(null)
  const latest = useRef({ folders, onHidden })
  useEffect(() => {
    latest.current = { folders, onHidden }
  })

  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    // Unread numbers in the sidebar move with hidden unread messages.
    void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
  }, [queryClient])

  useEffect(() => client.onDeletions(({ givenBack }) => {
    refresh()
    if (givenBack > 0) toast.error(t('mail:deleteGivenBack', { count: givenBack }))
  }), [client, refresh])

  const undo = useCallback(async (batchId: string) => {
    try {
      await client.undoDelete(batchId)
      refresh()
    } catch (error) {
      toast.error(t('mail:undoDeleteFailed'), { description: errorText(error) })
    }
  }, [client, refresh])

  const run = useCallback(async (email: EmailSummary, confirmed: boolean) => {
    try {
      const deletion = await client.deleteEmails([email.id], confirmed)
      latest.current.onHidden(email)
      refresh()
      toast.info(deletion.permanent ? t('mail:deletedForGood') : t('mail:movedToTrash'), {
        timeout: undoTimeout(deletion.undoUntil),
        action: { label: t('mail:undo'), onClick: () => void undo(deletion.batchId) },
      })
    } catch (error) {
      // The folder list was out of date; ask now rather than fail.
      if (error instanceof MailxClientError && error.failure.code === 'confirmationRequired' && !confirmed) {
        setConfirming({ kind: 'one', email })
        return
      }
      toast.error(t('mail:deleteFailed'), { description: errorText(error) })
    }
  }, [client, refresh, undo])

  const request = useCallback((email: EmailSummary) => {
    if (deletesForGood(email, latest.current.folders)) setConfirming({ kind: 'one', email })
    else void run(email, false)
  }, [run])

  // Several messages at once. Ids the host skips (already gone, already being
  // deleted) and local sent copies, which are never sent here, are reported back
  // as not done; one toast covers the whole batch and undoes all of it.
  const runBatch = useCallback(async (items: SelectedEmail[], confirmed: boolean, settle: BatchSettle) => {
    const local = items.filter((item) => item.localOnly).map((item) => item.id)
    const ids = items.filter((item) => !item.localOnly).map((item) => item.id).slice(0, SELECTION_LIMIT)
    const left = items.map((item) => item.id).filter((id) => !ids.includes(id) && !local.includes(id))
    if (ids.length === 0) {
      toast.error(t('mail:deleteLocalOnlyBatch'))
      settle({ done: [], failed: [...local, ...left] })
      return
    }
    try {
      const deletion = await client.deleteEmails(ids, confirmed, true)
      const skipped = new Set(deletion.skippedIds)
      const done = ids.filter((id) => !skipped.has(id))
      const failed = [...deletion.skippedIds, ...local, ...left]
      for (const id of done) latest.current.onHidden({ id })
      refresh()
      const permanentCount = items.filter((item) => done.includes(item.id) && deletesForGood(item, latest.current.folders)).length
      const title = failed.length > 0
        ? t('mail:batchDeletePartial', { count: done.length, failed: failed.length })
        : !deletion.permanent ? t('mail:batchMovedToTrash', { count: done.length })
          : permanentCount >= done.length ? t('mail:batchDeletedForGood', { count: done.length })
            : t('mail:batchDeleted', { count: done.length })
      toast.info(title, {
        timeout: undoTimeout(deletion.undoUntil),
        action: { label: t('mail:undo'), onClick: () => void undo(deletion.batchId) },
      })
      settle({ done, failed })
    } catch (error) {
      if (error instanceof MailxClientError && error.failure.code === 'confirmationRequired' && !confirmed) {
        setConfirming({ kind: 'batch', items, permanent: 0, settle })
        return
      }
      toast.error(t('mail:batchDeleteFailed', { count: items.length }), { description: errorText(error) })
      settle({ done: [], failed: items.map((item) => item.id) })
    }
  }, [client, refresh, undo])

  /** Deletes the selected messages, asking once first if any of them would be deleted for good. */
  const requestBatch = useCallback((items: SelectedEmail[], settle: BatchSettle) => {
    const permanent = items.filter((item) => !item.localOnly && deletesForGood(item, latest.current.folders)).length
    if (permanent > 0) setConfirming({ kind: 'batch', items, permanent, settle })
    else void runBatch(items, false, settle)
  }, [runBatch])

  /** Props for {@link DeleteForGoodDialog}, which asks before deleting for good. */
  const confirmation = {
    open: confirming !== null,
    count: confirming?.kind === 'batch' ? confirming.items.filter((item) => !item.localOnly).length : 1,
    permanentCount: confirming?.kind === 'batch' ? confirming.permanent : 1,
    onCancel: () => {
      if (confirming?.kind === 'batch') confirming.settle({ done: [], failed: confirming.items.map((item) => item.id) })
      setConfirming(null)
    },
    onConfirm: () => {
      const pending = confirming
      setConfirming(null)
      if (pending?.kind === 'one') void run(pending.email, true)
      else if (pending) void runBatch(pending.items, true, pending.settle)
    },
  }
  return { request, requestBatch, confirmation }
}

/** What a batch action finished, and what stays selected for another try. */
export type BatchResult = { done: string[]; failed: string[] }
export type BatchSettle = (result: BatchResult) => void

type Confirming =
  | { kind: 'one'; email: EmailSummary }
  | { kind: 'batch'; items: SelectedEmail[]; permanent: number; settle: BatchSettle }
