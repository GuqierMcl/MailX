import type { InfiniteData, QueryClient } from '@tanstack/react-query'
import type { Draft, DraftSummary, EmailDetail, EmailPage, OutboxItem } from '@/generated/accounts'
import { composeKeys, mailKeys } from '@/lib/query-client'
import type { loadMailTree } from './mail-tree'

type Removal = { accountId: string; draftIds: string[] }

/** Withdraw stale IPC replies before removing every cached copy of this account. */
export function clearRemovedAccount(queryClient: QueryClient, { accountId, draftIds }: Removal) {
  void queryClient.cancelQueries({ queryKey: mailKeys.all })
  void queryClient.cancelQueries({ queryKey: composeKeys.all })
  queryClient.setQueryData<Awaited<ReturnType<typeof loadMailTree>>>(mailKeys.tree(), (tree) => tree && ({
    accounts: tree.accounts.filter((account) => account.id !== accountId),
    folders: tree.folders.filter((folder) => folder.accountId !== accountId),
  }))
  queryClient.setQueriesData<InfiniteData<EmailPage>>({ queryKey: mailKeys.emailLists() }, (data) => data && ({
    ...data,
    pages: data.pages.map((page) => ({ ...page, items: page.items.filter((email) => email.accountId !== accountId) })),
  }))
  queryClient.setQueriesData<EmailPage>({ queryKey: [...mailKeys.all, 'palette'] }, (page) => page && ({
    ...page, items: page.items.filter((email) => email.accountId !== accountId),
  }))
  queryClient.removeQueries({
    queryKey: [...mailKeys.all, 'email'],
    predicate: (query) => {
      const detail = query.state.data as EmailDetail | undefined
      // A pending detail has no data yet, but its suffix still names its account.
      return detail?.summary.accountId === accountId ||
        (typeof query.queryKey[3] === 'string' && query.queryKey[3].startsWith(`[${JSON.stringify(accountId)},`))
    },
  })
  queryClient.setQueryData<DraftSummary[]>(composeKeys.drafts(), (rows) => rows?.filter((draft) =>
    draft.accountId !== accountId && !draftIds.includes(draft.id)))
  queryClient.setQueryData<OutboxItem[]>(composeKeys.outbox(), (rows) => rows?.filter((item) => item.accountId !== accountId))
  queryClient.removeQueries({
    queryKey: composeKeys.drafts(),
    predicate: (query) => query.queryKey.length > 2 && (
      draftIds.includes(String(query.queryKey[2])) || (query.state.data as Draft | undefined)?.accountId === accountId
    ),
  })
  void queryClient.invalidateQueries({ queryKey: mailKeys.all })
  void queryClient.invalidateQueries({ queryKey: composeKeys.all })
}
