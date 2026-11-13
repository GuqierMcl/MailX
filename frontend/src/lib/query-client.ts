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
  /** Every cached conversation; not under `emailLists`, whose entries are infinite-query pages. */
  conversations: () => [...mailKeys.all, 'conversation'] as const,
  /** The thread of the message the user opened (the "anchor"), whichever message of it is expanded. */
  conversation: (anchorId: string) => [...mailKeys.conversations(), anchorId] as const,
}

/** App preferences; sync rounds never touch them. */
export const settingsKeys = {
  all: ['settings'] as const,
}

/**
 * Drafts and the send queue. Sync rounds never touch them; `mailx://outbox` and
 * `mailx://drafts` events invalidate them (see `features/composer/compose-queries.ts`).
 */
export const composeKeys = {
  all: ['compose'] as const,
  drafts: () => [...composeKeys.all, 'drafts'] as const,
  outbox: () => [...composeKeys.all, 'outbox'] as const,
}

/**
 * Sender grouping rules (contact aggregation). Not under `mailKeys.all`: sync
 * rounds do not change them, so they are refetched only after the user edits one.
 */
export const contactKeys = {
  all: ['contacts'] as const,
  aggregations: () => [...contactKeys.all, 'aggregations'] as const,
}

/** Which translation services have a saved key, and what the saved translations take up. Only the user's own edits change them. */
export const translationKeys = {
  all: ['translation'] as const,
  keys: () => [...translationKeys.all, 'keys'] as const,
  cache: () => [...translationKeys.all, 'cache'] as const,
  proxyPassword: () => [...translationKeys.all, 'proxyPassword'] as const,
}

/** The AI settings, the provider catalog, each connection's model list and downloaded provider icons. Model lists are never persisted. */
export const aiKeys = {
  all: ['ai'] as const,
  state: () => [...aiKeys.all, 'state'] as const,
  catalog: () => [...aiKeys.all, 'catalog'] as const,
  models: (connectionId: string) => [...aiKeys.all, 'models', connectionId] as const,
  icon: (providerId: string) => [...aiKeys.all, 'icon', providerId] as const,
}
