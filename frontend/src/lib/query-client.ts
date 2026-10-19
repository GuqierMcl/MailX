import { QueryClient } from '@tanstack/react-query'
import type { EmailListQuery } from '@/generated/accounts'

/**
 * Data comes from the local core over IPC, and sync events say when it changed,
 * so queries never go stale on a timer, never refetch on focus and never retry
 * (a local failure will not fix itself a moment later).
 */
export function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { staleTime: Infinity, retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  })
}

/** Everything under `all` is mail data that a sync round may change. */
export const mailKeys = {
  all: ['mail'] as const,
  tree: () => [...mailKeys.all, 'tree'] as const,
  emailLists: () => [...mailKeys.all, 'emails'] as const,
  /** `sources` pins the accounts' server identities so a server change starts a fresh list. */
  emails: (sources: string, query: EmailListQuery) => [...mailKeys.emailLists(), sources, query] as const,
  email: (id: string) => [...mailKeys.all, 'email', id] as const,
}
