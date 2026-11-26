import { useCallback, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from '@/components/ui/toast'
import type { OverviewSender } from '@/generated/accounts'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { emailQuery } from '@/features/mail/format'

/**
 * "Mark all as read" for one sender of the unified inbox: the host names the unread mail from the
 * address (the same query the list's "select all" uses), the existing bulk flag write-back marks
 * it, and a toast offers Undo, which marks the same messages unread again. At most the 500 a
 * selection holds are marked at once; the overview then shows what is left.
 */
export function useMarkSenderRead(client: MailxClient) {
  const queryClient = useQueryClient()
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set())
  const running = useRef(new Set<string>())

  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.overviews() })
  }, [queryClient])

  const markRead = useCallback(async (sender: OverviewSender) => {
    if (running.current.has(sender.address)) return
    running.current.add(sender.address)
    setBusy(new Set(running.current))
    try {
      const selection = await client.selectEmails(emailQuery({ folderRole: 'inbox', archive: 'exclude', unread: true, fromAddress: sender.address }))
      const ids = selection.items.filter((item) => !item.isRead && !item.localOnly).map((item) => item.id)
      if (ids.length === 0) {
        refresh()
        return
      }
      const outcome = await client.setFlags(ids, 'read', true)
      const done = outcome.results.filter((result) => result.outcome !== 'failed').map((result) => result.emailId)
      const failed = ids.length - done.length
      refresh()
      if (done.length === 0) {
        toast.error(t('mail:changeReadFailed'))
        return
      }
      toast.info(failed > 0 ? t('mail:batchFlagPartial', { count: done.length, failed }) : t('mail:batchMarkedRead', { count: done.length }), {
        action: {
          label: t('now:undo'),
          onClick: () => {
            client.setFlags(done, 'read', false).then(refresh, (error: unknown) => {
              refresh()
              toast.error(t('mail:changeReadFailed'), { description: errorText(error) })
            })
          },
        },
      })
    } catch (error) {
      refresh()
      toast.error(t('mail:changeReadFailed'), { description: errorText(error) })
    } finally {
      running.current.delete(sender.address)
      setBusy(new Set(running.current))
    }
  }, [client, refresh])

  return { markRead, busy }
}
