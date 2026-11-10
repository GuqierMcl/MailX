import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useCallback, useEffect } from 'react'
import { useQueryClient, type InfiniteData } from '@tanstack/react-query'
import { toast } from '@/components/ui/toast'
import type { EmailDetail, EmailPage, EmailSummary, SelectedEmail } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import type { BatchSettle } from './use-delete-email'
import { SELECTION_LIMIT } from './use-list-selection'

type Flag = 'read' | 'starred'

/** What the user is told when a star can only be kept on this device. */
const LOCAL_STAR_TITLE = 'mail:localStarTitle'
const LOCAL_STAR_BODY = 'mail:localStarDescription'

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
 * Applies `change` to the reader's own copy of a message, wherever it is cached.
 * The reader no longer borrows its flags from the list, and the list may not even
 * contain the message (a filter, another mailbox), so it is updated directly.
 */
function patchOpened(
  client: ReturnType<typeof useQueryClient>,
  emailId: string,
  change: (summary: EmailSummary) => EmailSummary,
) {
  client.setQueriesData<EmailDetail>({ queryKey: mailKeys.email(emailId) }, (detail) =>
    detail && { ...detail, summary: change(detail.summary) })
}

/** `mailKeys.email(id)` entries, as opposed to lists and the folder tree. */
const isEmailDetail = (query: { queryKey: readonly unknown[] }) => query.queryKey[1] === 'email'

/** Both places a message's flags are shown: the list rows and the open reader. */
function patchEverywhere(
  client: ReturnType<typeof useQueryClient>,
  emailId: string,
  change: (summary: EmailSummary) => EmailSummary,
) {
  patchLists(client, emailId, change)
  patchOpened(client, emailId, change)
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
    patchEverywhere(queryClient, email.id, (item) => ({ ...item, [field]: wanted, pendingChanges: true }))
    // The sidebar's numbers live in the folder tree, a separate query that only a
    // sync event refreshes. The core moves the folder's unread number in the same
    // step as the change, so re-reading it (a local read, no network) is all it takes.
    // Done on failure too, since the core may have changed it before refusing.
    const refreshCounts = () => {
      if (flag === 'read') void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
    }
    try {
      const outcome = await client.setFlag(email.id, flag, wanted)
      refreshCounts()
      if (outcome === 'localOnly') {
        // Not pending: it is final. Tell the user, once for this change.
        patchEverywhere(queryClient, email.id, (item) => ({ ...item, pendingChanges: false }))
        if (wanted) toast.info(t(LOCAL_STAR_TITLE), { description: t(LOCAL_STAR_BODY), timeout: 9000 })
      }
    } catch (error) {
      refreshCounts()
      patchEverywhere(queryClient, email.id, (item) => ({ ...item, [field]: before, pendingChanges: false }))
      toast.error(flag === 'read' ? t('mail:changeReadFailed') : t('mail:changeStarFailed'), { description: errorText(error) })
    }
  }, [client, queryClient])

  /**
   * The same change for the list selection, in one call. Every row shows the new
   * value at once; a message the host could not change gets its old value back
   * and stays selected. One toast reports the batch (no undo), carrying the
   * local-star notice when some stars stay on this device.
   */
  const setFlags = useCallback(async (items: SelectedEmail[], flag: Flag, wanted: boolean, settle: BatchSettle) => {
    const field = FIELD[flag]
    const local = items.filter((item) => item.localOnly).map((item) => item.id)
    const targets = items.filter((item) => !item.localOnly).slice(0, SELECTION_LIMIT)
    const left = items.filter((item) => !item.localOnly).slice(SELECTION_LIMIT).map((item) => item.id)
    const changing = targets.filter((item) => item[field] !== wanted)
    for (const item of changing) patchEverywhere(queryClient, item.id, (row) => ({ ...row, [field]: wanted, pendingChanges: true }))
    const restore = (ids: string[]) => {
      const before = new Map(changing.map((item) => [item.id, item[field]]))
      for (const id of ids) {
        if (before.has(id)) patchEverywhere(queryClient, id, (row) => ({ ...row, [field]: before.get(id), pendingChanges: false }))
      }
    }
    const refreshCounts = () => {
      if (flag === 'read') void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
    }
    const failedTitle = flag === 'read' ? t('mail:changeReadFailed') : t('mail:changeStarFailed')
    if (targets.length === 0) {
      toast.error(failedTitle)
      settle({ done: [], failed: items.map((item) => item.id) })
      return
    }
    let outcome
    try {
      outcome = await client.setFlags(targets.map((item) => item.id), flag, wanted)
    } catch (error) {
      refreshCounts()
      restore(changing.map((item) => item.id))
      toast.error(failedTitle, { description: errorText(error) })
      settle({ done: [], failed: items.map((item) => item.id) })
      return
    }
    refreshCounts()
    const failedIds = outcome.results.filter((result) => result.outcome === 'failed').map((result) => result.emailId)
    restore(failedIds)
    const localStars = outcome.results.filter((result) => result.outcome === 'localOnly')
    // Not pending: a local-only star is final.
    for (const result of localStars) patchEverywhere(queryClient, result.emailId, (row) => ({ ...row, pendingChanges: false }))
    const failed = [...failedIds, ...local, ...left]
    const done = targets.map((item) => item.id).filter((id) => !failedIds.includes(id))
    const title = failed.length > 0
      ? t('mail:batchFlagPartial', { count: done.length, failed: failed.length })
      : t(flag === 'read' ? (wanted ? 'mail:batchMarkedRead' : 'mail:batchMarkedUnread') : (wanted ? 'mail:batchStarred' : 'mail:batchUnstarred'), { count: done.length })
    const localNotice = flag === 'starred' && wanted && localStars.length > 0
    toast.info(title, localNotice ? { description: t(LOCAL_STAR_BODY), timeout: 9000 } : undefined)
    settle({ done, failed })
  }, [client, queryClient])

  // A write-back round that confirmed something clears the "waiting" markers and
  // may carry stars that turned out to be local-only.
  useEffect(() => client.onFlagWriteBack((written) => {
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    // The open message holds its own copy of "waiting"; re-read it as well.
    void queryClient.invalidateQueries({ queryKey: mailKeys.all, predicate: isEmailDetail })
    if (written.localOnlyStars > 0) {
      toast.info(t(LOCAL_STAR_TITLE), { description: t(LOCAL_STAR_BODY), timeout: 9000 })
    }
  }), [client, queryClient])

  return { setFlag, setFlags }
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
