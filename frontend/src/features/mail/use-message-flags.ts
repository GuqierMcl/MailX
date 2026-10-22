import { useCallback, useEffect } from 'react'
import { useQueryClient, type InfiniteData } from '@tanstack/react-query'
import { toast } from '@/components/ui/toast'
import type { EmailPage, EmailSummary } from '@/generated/accounts'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'

type Flag = 'read' | 'starred'

/** What the user is told when a star can only be kept on this device. */
const LOCAL_STAR_TITLE = '星标仅保存在本机'
const LOCAL_STAR_BODY = '该邮箱服务商不支持在服务器上保存星标，换一台设备将看不到它。已读状态不受影响，仍会同步。'

/** `EmailSummary` fields each flag drives. */
const FIELD = { read: 'isRead', starred: 'isStarred' } as const

/** Applies `change` to the listed copy of one message in every cached list page. */
function patchLists(
  client: ReturnType<typeof useQueryClient>,
  emailId: string,
  change: (summary: EmailSummary) => EmailSummary,
) {
  client.setQueriesData<InfiniteData<EmailPage>>({ queryKey: mailKeys.emailLists() }, (data) =>
    data && {
      ...data,
      pages: data.pages.map((page) => ({
        ...page,
        items: page.items.map((item) => (item.id === emailId ? change(item) : item)),
      })),
    })
}

/**
 * Read and starred changes. The list shows the new value at once (that is where
 * the reader gets these flags from) and the server is written in the background.
 * A rejected change puts the old value back and says so.
 *
 * One notice per starred change that stays local, because it would otherwise be
 * silently different from what the other devices show.
 */
export function useMessageFlags(client: MailxClient) {
  const queryClient = useQueryClient()

  const setFlag = useCallback(async (email: EmailSummary, flag: Flag, wanted: boolean) => {
    const field = FIELD[flag]
    const before = email[field]
    if (before === wanted) return
    patchLists(queryClient, email.id, (item) => ({ ...item, [field]: wanted, pendingChanges: true }))
    try {
      const outcome = await client.setFlag(email.id, flag, wanted)
      if (outcome === 'localOnly') {
        // Not pending: it is final. Tell the user, once for this change.
        patchLists(queryClient, email.id, (item) => ({ ...item, pendingChanges: false }))
        if (wanted) toast.info(LOCAL_STAR_TITLE, { description: LOCAL_STAR_BODY, timeout: 9000 })
      }
    } catch (error) {
      patchLists(queryClient, email.id, (item) => ({ ...item, [field]: before, pendingChanges: false }))
      toast.error(flag === 'read' ? '无法更改已读状态' : '无法更改星标', { description: clientFailure(error).message })
    }
  }, [client, queryClient])

  // A write-back round that confirmed something clears the "waiting" markers and
  // may carry stars that turned out to be local-only.
  useEffect(() => client.onFlagWriteBack((written) => {
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    if (written.localOnlyStars > 0) {
      toast.info(LOCAL_STAR_TITLE, { description: LOCAL_STAR_BODY, timeout: 9000 })
    }
  }), [client, queryClient])

  return { setFlag }
}

/**
 * Marks the open message read, once per opening. It waits until the message is
 * actually shown (detail loaded, tab visible) so a quick pass through the list
 * with the arrow keys does not mark everything, and it never touches a message
 * that is already read or whose change is still waiting.
 */
export function useMarkOpenedAsRead(
  email: EmailSummary | null | undefined,
  shown: boolean,
  onFlag: (email: EmailSummary, flag: 'read', wanted: boolean) => void,
  delayMs = 0,
) {
  const id = email?.id
  const unread = email ? !email.isRead : false
  const waiting = email?.pendingChanges ?? false
  useEffect(() => {
    if (!id || !unread || !shown || waiting) return
    const timer = setTimeout(() => { if (email) onFlag(email, 'read', true) }, delayMs)
    return () => clearTimeout(timer)
    // Once per opened message: only its identity and whether it is shown decide.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, shown])
}
