import { useCallback, useEffect, useRef } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import type { EmailSummary, Folder, SelectedEmail } from '@/generated/accounts'
import { toast } from '@/components/ui/toast'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import type { BatchSettle } from './use-delete-email'
import { SELECTION_LIMIT } from './use-list-selection'

/** Whether `email` is an inbox message, the only kind that can be archived locally. */
export function canArchive(email: Pick<EmailSummary, 'folderId' | 'localOnly'>, folders: Folder[]) {
  return !email.localOnly && folders.find((folder) => folder.id === email.folderId)?.role === 'inbox'
}

/** The shared archive call: changes the emails, then refreshes lists, counts and opened copies. */
function useArchiveChange(client: MailxClient) {
  const queryClient = useQueryClient()
  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    // The inbox and archived unread numbers move together.
    void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
  }, [queryClient])

  const change = useCallback(async (emailIds: string[], archive: boolean, partial?: boolean) => {
    const receipt = await (partial
      ? archive ? client.archiveEmails(emailIds, true) : client.unarchiveEmails(emailIds, true)
      : archive ? client.archiveEmails(emailIds) : client.unarchiveEmails(emailIds))
    // An opened copy carries the archived mark.
    for (const id of emailIds) void queryClient.invalidateQueries({ queryKey: mailKeys.email(id) })
    refresh()
    return receipt
  }, [client, queryClient, refresh])

  const undo = useCallback((undoToken: string, emailIds: string[]) => {
    void client.undoArchive(undoToken).then(() => {
      for (const id of emailIds) void queryClient.invalidateQueries({ queryKey: mailKeys.email(id) })
      refresh()
    }).catch((error: unknown) => {
      toast.error(t('mail:undoArchiveFailed'), { description: errorText(error) })
    })
  }, [client, queryClient, refresh])

  return { change, undo }
}

/**
 * Archives and unarchives inbox emails on this device only; the server is not
 * touched. The toast restores exactly this change through a host receipt.
 * `onHidden` runs once an email left the view, so the reader can let go of it.
 */
export function useArchiveEmail(client: MailxClient, onHidden: (email: EmailSummary) => void) {
  const { change, undo } = useArchiveChange(client)
  const latest = useRef(onHidden)
  useEffect(() => {
    latest.current = onHidden
  })

  return useCallback(async (email: EmailSummary, archive: boolean) => {
    let receipt
    try {
      receipt = await change([email.id], archive)
    } catch (error) {
      toast.error(t(archive ? 'mail:archiveFailed' : 'mail:unarchiveFailed'), { description: errorText(error) })
      return
    }
    latest.current(email)
    if (receipt.changedCount === 0) return
    const undoToken = receipt.undoToken
    toast.info(t(archive ? 'mail:archived' : 'mail:movedBackToInbox'), {
      action: undoToken ? { label: t('mail:undo'), onClick: () => undo(undoToken, [email.id]) } : undefined,
    })
  }, [change, undo])
}

/**
 * The list selection's archive and "move back to inbox": one partial call for
 * up to 500 messages. Messages that cannot change (not inbox mail, local
 * copies, gone meanwhile) are reported back as not done; one toast covers the
 * batch, and its undo restores everything the call changed.
 */
export function useArchiveEmails(client: MailxClient, onHidden: (email: { id: string }) => void) {
  const { change, undo } = useArchiveChange(client)
  const latest = useRef(onHidden)
  useEffect(() => {
    latest.current = onHidden
  })

  return useCallback(async (items: SelectedEmail[], archive: boolean, applicable: (item: SelectedEmail) => boolean, settle: BatchSettle) => {
    const ids = items.filter(applicable).map((item) => item.id).slice(0, SELECTION_LIMIT)
    const left = items.map((item) => item.id).filter((id) => !ids.includes(id))
    if (ids.length === 0) {
      settle({ done: [], failed: left })
      return
    }
    let receipt
    try {
      receipt = await change(ids, archive, true)
    } catch (error) {
      toast.error(t(archive ? 'mail:archiveFailed' : 'mail:unarchiveFailed'), { description: errorText(error) })
      settle({ done: [], failed: items.map((item) => item.id) })
      return
    }
    const skipped = new Set(receipt.skippedIds)
    const done = ids.filter((id) => !skipped.has(id))
    const failed = [...receipt.skippedIds, ...left]
    for (const id of done) latest.current({ id })
    const undoToken = receipt.undoToken
    const title = failed.length > 0
      ? t(archive ? 'mail:batchArchivePartial' : 'mail:batchUnarchivePartial', { count: done.length, failed: failed.length })
      : t(archive ? 'mail:batchArchived' : 'mail:batchUnarchived', { count: done.length })
    toast.info(title, {
      action: undoToken ? { label: t('mail:undo'), onClick: () => undo(undoToken, done) } : undefined,
    })
    settle({ done, failed })
  }, [change, undo])
}
