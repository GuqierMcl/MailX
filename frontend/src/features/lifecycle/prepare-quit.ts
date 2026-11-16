import type { QueryClient } from '@tanstack/react-query'
import { flushPendingLayoutSaves } from '@/features/settings/pending-layout-saves'

type QuitSave = () => Promise<void>

/** The inline composer could not write its draft; the quit is cancelled and the editor stays. */
export class InlineDraftUnsaved extends Error {
  constructor() {
    super('inline draft not saved')
    this.name = 'InlineDraftUnsaved'
  }
}
const quitSaves = new WeakMap<QueryClient, Set<QuitSave>>()

/**
 * Work the main window must finish before an explicit quit, such as writing the
 * inline composer's draft (sending.md section 17.7). A rejection cancels the quit.
 */
export function registerQuitSave(client: QueryClient, save: QuitSave): () => void {
  let registered = quitSaves.get(client)
  if (!registered) { registered = new Set(); quitSaves.set(client, registered) }
  registered.add(save)
  return () => { registered.delete(save) }
}

async function runQuitSaves(client: QueryClient): Promise<void> {
  const results = await Promise.allSettled([...quitSaves.get(client) ?? []].map((save) => save()))
  const failure = results.find((result) => result.status === 'rejected')
  if (failure?.status === 'rejected') throw failure.reason
}

/** Waits for registered quit saves and pending main-window mutations, including layout saves started by this flush. */
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
    void Promise.all([flushPendingLayoutSaves(client), runQuitSaves(client)])
      .then(() => { flushed = true; inspect() }, (cause) => finish(false, cause))
  })
}
