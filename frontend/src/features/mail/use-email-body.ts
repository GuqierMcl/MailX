import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useIsMutating, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { Account, AccountSyncState, EmailDetail, EmailSummary } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'

// The prefix still supports invalidating email(id); the suffix prevents an old
// account revision or worker generation from populating the new reader's cache.
export function bodyIdentity(account: Account, state?: AccountSyncState) {
  return JSON.stringify([account.id, account.mailboxSourceId, account.revision,
    state?.mailboxSourceId === account.mailboxSourceId ? state.generation : 0])
}

/**
 * The message the reader shows. It is only an id and the account it belongs to:
 * the reader reads everything else, flags included, from the message itself, so
 * it never depends on whether the list currently contains this message.
 */
export type OpenedEmail = Pick<EmailSummary, 'id' | 'accountId'>

export function useEmailBody(client: MailxClient, opened: OpenedEmail | null,
  accounts: Account[], states: Record<string, AccountSyncState>) {
  const queryClient = useQueryClient()
  const [removedAccounts, setRemovedAccounts] = useState(() => new Set<string>())
  const identities = new Map(accounts.filter((account) => !removedAccounts.has(account.id))
    .map((account) => [account.id, bodyIdentity(account, states[account.id])]))
  const identity = opened ? identities.get(opened.accountId) ?? '' : ''
  const id = opened?.id ?? ''
  const key = useMemo(() => [...mailKeys.email(id), identity] as const, [id, identity])
  const attempted = useRef(new Set<string>())
  const flights = useRef(new Set<string>())
  const current = useRef(identities)
  useLayoutEffect(() => { current.current = identities })
  useEffect(() => () => { current.current = new Map() }, [])
  useEffect(() => client.onAccountRemoved(({ accountId }) => {
    // An in-flight fetch can finish before the tree update renders. Revoke its
    // authority synchronously so it cannot repopulate a removed account's cache.
    current.current.delete(accountId)
    setRemovedAccounts((previous) => new Set(previous).add(accountId))
  }), [client])

  const detail = useQuery({
    queryKey: key,
    queryFn: () => client.getEmail(id),
    enabled: Boolean(id && identity),
  })
  const { mutate } = useMutation({
    mutationKey: ['body-download'],
    mutationFn: async (input: { email: EmailDetail; identity: string; key: typeof key; flight: string }) => {
      await queryClient.cancelQueries({ queryKey: input.key, exact: true })
      return client.fetchBody(input.email.summary.id)
    },
    onSuccess: async (email, input) => {
      await queryClient.cancelQueries({ queryKey: input.key, exact: true })
      if (current.current.get(input.email.summary.accountId) !== input.identity ||
        email.summary.id !== input.email.summary.id || email.summary.accountId !== input.email.summary.accountId ||
        email.summary.folderId !== input.email.summary.folderId) return
      queryClient.setQueryData(input.key, email)
      // Invalidation retains all loaded infinite-query pages and their cursors.
      await queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    },
    onError: async (_error, input) => {
      await queryClient.cancelQueries({ queryKey: input.key, exact: true })
      if (current.current.get(input.email.summary.accountId) !== input.identity) return
      queryClient.setQueryData(input.key, {
        ...input.email, summary: { ...input.email.summary, bodyState: 'failed' }, bodyFailure: null,
      } satisfies EmailDetail)
    },
    onSettled: (_data, _error, input) => { flights.current.delete(input.flight) },
  })
  const download = useCallback(() => {
    if (!detail.data || !identity) return
    const flight = JSON.stringify(key)
    if (flights.current.has(flight)) return
    attempted.current.add(flight)
    flights.current.add(flight)
    mutate({ email: detail.data, identity, key, flight })
  }, [detail.data, identity, key, mutate])
  useEffect(() => {
    const state = detail.data?.summary.bodyState
    if ((state === 'missing' || state === 'fetching') && !attempted.current.has(JSON.stringify(key))) download()
  }, [detail.data, key, download])
  const pending = useIsMutating({
    mutationKey: ['body-download'],
    predicate: (mutation) => (mutation.state.variables as { flight?: string } | undefined)?.flight === JSON.stringify(key),
  }) > 0

  // The message is the single source for what the reader shows, flags included:
  // the core returns the same read, starred and waiting-change values the list
  // gets. A change made elsewhere reaches it through `refreshOpened`.
  const email = detail.data ?? null
  return { detail, email, identity, pending, download }
}
