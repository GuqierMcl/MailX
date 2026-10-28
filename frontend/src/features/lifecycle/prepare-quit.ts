import type { QueryClient } from '@tanstack/react-query'
import { flushPendingLayoutSaves } from '@/features/settings/pending-layout-saves'

/** Waits for pending main-window mutations, including layout saves started by this flush. */
export function prepareMainWindowQuit(client: QueryClient, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) { reject(new DOMException('Quit cancelled', 'AbortError')); return }
    const cache = client.getMutationCache()
    const observed = new Set(cache.getAll().filter((mutation) => mutation.state.status === 'pending'))
    let flushed = false
    let settled = false
    let hasFailure = false
    let failure: unknown
    const finish = (success: boolean, error?: unknown) => {
      if (settled) return
      settled = true
      stop()
      signal.removeEventListener('abort', abort)
      if (success) resolve()
      else reject(error)
    }
    const inspect = () => {
      for (const mutation of cache.getAll()) if (mutation.state.status === 'pending') observed.add(mutation)
      for (const mutation of observed) {
        if (mutation.state.status === 'error') {
          if (!hasFailure) failure = mutation.state.error
          hasFailure = true
          observed.delete(mutation)
        }
        else if (mutation.state.status === 'success') observed.delete(mutation)
      }
      if (flushed && observed.size === 0) finish(!hasFailure, failure)
    }
    const stop = cache.subscribe(inspect)
    const abort = () => finish(false, new DOMException('Quit cancelled', 'AbortError'))
    signal.addEventListener('abort', abort, { once: true })
    void flushPendingLayoutSaves(client).then(() => { flushed = true; inspect() }, (cause) => finish(false, cause))
  })
}
