import { useCallback, useEffect, useRef } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import type { EmailSummary, Folder } from '@/generated/accounts'
import { toast } from '@/components/ui/toast'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'

/** Whether `email` is an inbox message, the only kind that can be archived locally. */
export function canArchive(email: EmailSummary, folders: Folder[]) {
  return !email.localOnly && folders.find((folder) => folder.id === email.folderId)?.role === 'inbox'
}

/**
 * Archives and unarchives inbox emails on this device only; the server is not
 * touched. The toast restores exactly this change through a host receipt.
 * `onHidden` runs once an email left the view, so the reader can let go of it.
 */
export function useArchiveEmail(client: MailxClient, onHidden: (email: EmailSummary) => void) {
  const queryClient = useQueryClient()
  const latest = useRef(onHidden)
  useEffect(() => {
    latest.current = onHidden
  })

  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    // The inbox and archived unread numbers move together.
    void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
  }, [queryClient])

  const change = useCallback(async (emailIds: string[], archive: boolean) => {
    const receipt = await (archive ? client.archiveEmails(emailIds) : client.unarchiveEmails(emailIds))
    // An opened copy carries the archived mark.
    for (const id of emailIds) void queryClient.invalidateQueries({ queryKey: mailKeys.email(id) })
    refresh()
    return receipt
  }, [client, queryClient, refresh])

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
      action: undoToken ? {
        label: t('mail:undo'),
        onClick: () => void client.undoArchive(undoToken).then(() => {
          void queryClient.invalidateQueries({ queryKey: mailKeys.email(email.id) })
          refresh()
        }).catch((error: unknown) => {
          toast.error(t('mail:undoArchiveFailed'), { description: errorText(error) })
        }),
      } : undefined,
    })
  }, [change, client, queryClient, refresh])
}
