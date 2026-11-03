import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from '@/components/ui/toast'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'

/** Agent writes and UI writes share the same committed local state events. */
export function useMailStateChanges(client: MailxClient) {
  const queryClient = useQueryClient()

  useEffect(() => client.onMailStateChanged((change) => {
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
    // A shared Message-ID can affect more rows than fit in the event's ID
    // sample; always refresh cached details as well as lists and counts.
    void queryClient.invalidateQueries({
      queryKey: mailKeys.all,
      predicate: (query) => query.queryKey[1] === 'email',
    })

    // UI actions already show their own notice. Only actual agent archive
    // changes need another explanation for messages leaving the current list.
    if (change.source === 'user' || change.changedCount === 0
      || (change.kind !== 'archive' && change.kind !== 'unarchive')) return
    const undoToken = change.undoToken
    toast.info(t(change.kind === 'archive' ? 'mail:agentArchived' : 'mail:agentUnarchived', {
      source: change.source === 'mcp' ? 'MCP' : 'AI',
      count: change.changedCount,
    }), {
      timeout: 9000,
      action: undoToken ? {
        label: t('mail:undo'),
        onClick: () => void client.undoArchive(undoToken).catch((error: unknown) => {
          toast.error(t('mail:undoArchiveFailed'), { description: errorText(error) })
        }),
      } : undefined,
    })
  }), [client, queryClient])
}
