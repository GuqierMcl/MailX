import { useCallback, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from '@/components/ui/toast'
import type { EmailSummary } from '@/generated/accounts'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'

/**
 * Single-message quick actions on the Now page's unread list: mark read, toggle
 * star, and local archive, each using the host's existing write-back and undo flows.
 */
export function useNowMailActions(client: MailxClient) {
  const queryClient = useQueryClient()
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set())
  const running = useRef(new Set<string>())

  const refresh = useCallback((emailId?: string) => {
    if (emailId) void queryClient.invalidateQueries({ queryKey: mailKeys.email(emailId) })
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.conversations() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.overviews() })
  }, [queryClient])

  const run = useCallback(async (emailId: string, work: () => Promise<void>) => {
    if (running.current.has(emailId)) return
    running.current.add(emailId)
    setBusy(new Set(running.current))
    try {
      await work()
    } finally {
      running.current.delete(emailId)
      setBusy(new Set(running.current))
    }
  }, [])

  const markRead = useCallback((email: EmailSummary) => run(email.id, async () => {
    const wanted = !email.isRead
    try {
      const outcome = await client.setFlags([email.id], 'read', wanted)
      refresh(email.id)
      const ok = outcome.results.some((result) => result.outcome !== 'failed')
      if (!ok) {
        toast.error(t('mail:changeReadFailed'))
        return
      }
      if (wanted) {
        toast.info(t('now:markedRead'), {
          action: {
            label: t('now:undo'),
            onClick: () => {
              client.setFlags([email.id], 'read', false).then(() => refresh(email.id), (error: unknown) => {
                refresh(email.id)
                toast.error(t('mail:changeReadFailed'), { description: errorText(error) })
              })
            },
          },
        })
      }
    } catch (error) {
      refresh(email.id)
      toast.error(t('mail:changeReadFailed'), { description: errorText(error) })
    }
  }), [client, refresh, run])

  const toggleStar = useCallback((email: EmailSummary) => run(email.id, async () => {
    try {
      const outcome = await client.setFlags([email.id], 'starred', !email.isStarred)
      refresh(email.id)
      if (!outcome.results.some((result) => result.outcome !== 'failed')) {
        toast.error(t('mail:changeStarFailed'))
      }
    } catch (error) {
      refresh(email.id)
      toast.error(t('mail:changeStarFailed'), { description: errorText(error) })
    }
  }), [client, refresh, run])

  const archive = useCallback((email: EmailSummary) => run(email.id, async () => {
    try {
      const receipt = await client.archiveEmails([email.id])
      refresh(email.id)
      if (receipt.changedCount === 0) return
      const undoToken = receipt.undoToken
      toast.info(t('mail:archived'), {
        action: undoToken ? {
          label: t('mail:undo'),
          onClick: () => {
            client.undoArchive(undoToken).then(() => refresh(email.id), (error: unknown) => {
              refresh(email.id)
              toast.error(t('mail:undoArchiveFailed'), { description: errorText(error) })
            })
          },
        } : undefined,
      })
    } catch (error) {
      refresh(email.id)
      toast.error(t('mail:archiveFailed'), { description: errorText(error) })
    }
  }), [client, refresh, run])

  return { markRead, toggleStar, archive, busy }
}
