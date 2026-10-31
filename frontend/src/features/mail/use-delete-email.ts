import { useCallback, useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import type { EmailSummary, Folder } from '@/generated/accounts'
import { toast } from '@/components/ui/toast'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'

/**
 * Whether deleting `email` expunges it for good instead of moving it to Trash:
 * it already is in Trash, or its account has none. The core makes the same
 * call and refuses a permanent delete the user did not confirm.
 */
export function deletesForGood(email: EmailSummary, folders: Folder[]) {
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
export function useDeleteEmail(client: MailxClient, folders: Folder[], onHidden: (email: EmailSummary) => void) {
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState<EmailSummary | null>(null)
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
        setConfirming(email)
        return
      }
      toast.error(t('mail:deleteFailed'), { description: errorText(error) })
    }
  }, [client, refresh, undo])

  const request = useCallback((email: EmailSummary) => {
    if (deletesForGood(email, latest.current.folders)) setConfirming(email)
    else void run(email, false)
  }, [run])

  /** Props for {@link DeleteForGoodDialog}, which asks before deleting for good. */
  const confirmation = {
    open: confirming !== null,
    onCancel: () => setConfirming(null),
    onConfirm: () => {
      const email = confirming
      setConfirming(null)
      if (email) void run(email, true)
    },
  }
  return { request, confirmation }
}
