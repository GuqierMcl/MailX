import type { QueryClient } from '@tanstack/react-query'
import type { MailxClient } from '@/lib/mailx-client'
import { acceptSettingsSnapshot } from '@/lib/settings-cache'

type Watch = { ready?: Promise<void>; stop?: () => void; disposed: boolean; owners: number }
const watches = new WeakMap<QueryClient, Watch>()

/** Reading waits for registration so an update cannot fall between subscribe and get. */
export function ensureSettingsWatch(client: MailxClient, queryClient: QueryClient): Promise<void> {
  const existing = watches.get(queryClient)
  if (existing?.ready) return existing.ready
  const watch: Watch = existing ?? { disposed: false, owners: 0 }
  watches.set(queryClient, watch)
  watch.ready = client.onSettingsChanged(() => {
    void client.getSettings().then((snapshot) => {
      if (!watch.disposed) acceptSettingsSnapshot(queryClient, snapshot)
    }).catch(() => undefined)
  }).then((stop) => {
    if (watch.disposed) stop()
    else watch.stop = stop
  }).catch(() => { if (!watch.disposed) watch.ready = undefined })
  return watch.ready
}

export function retainSettingsWatch(client: MailxClient, queryClient: QueryClient): () => void {
  void ensureSettingsWatch(client, queryClient)
  const watch = watches.get(queryClient)
  if (!watch) return () => undefined
  watch.owners += 1
  return () => {
    watch.owners -= 1
    // StrictMode remounts synchronously; keep its registered listener alive.
    queueMicrotask(() => {
      if (watch.owners || watches.get(queryClient) !== watch) return
      watch.disposed = true
      watch.stop?.()
      watches.delete(queryClient)
    })
  }
}
