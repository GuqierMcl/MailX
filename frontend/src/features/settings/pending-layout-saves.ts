import type { QueryClient } from '@tanstack/react-query'

type Flush = () => Promise<void>
const writers = new WeakMap<QueryClient, Set<Flush>>()

/** Each window's query client owns its outstanding layout writes. */
export function registerPendingLayoutSave(client: QueryClient, flush: Flush): () => void {
  let registered = writers.get(client)
  if (!registered) { registered = new Set(); writers.set(client, registered) }
  registered.add(flush)
  return () => { registered.delete(flush) }
}

/** Rejects on failure; a quit must never mistake a failed write for a saved layout. */
export async function flushPendingLayoutSaves(client: QueryClient): Promise<void> {
  const results = await Promise.allSettled([...writers.get(client) ?? []].map((flush) => flush()))
  const failure = results.find((result) => result.status === 'rejected')
  if (failure?.status === 'rejected') throw failure.reason
}
