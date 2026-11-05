import { useCallback, useEffect, useRef, useState } from 'react'
import { toast } from '@/components/ui/toast'
import { errorText } from '@/i18n/errors'
import type { AccountSyncState } from '@/generated/accounts'

const START_TIMEOUT_MS = 15_000

type Request = { baseline: AccountSyncState | undefined }
type ActiveRequest = { timer: ReturnType<typeof setTimeout> | undefined }

function finished(request: Request, sync: AccountSyncState | undefined) {
  const baseline = request.baseline
  if (!sync) return Boolean(baseline)
  if (baseline && (sync.accountId !== baseline.accountId || sync.mailboxSourceId !== baseline.mailboxSourceId)) return true
  if (baseline && sync.generation !== baseline.generation) {
    // An explicit provider-policy retry intentionally starts a new worker.
    return baseline.failure?.code !== 'providerPolicy' || sync.generation < baseline.generation || sync.phase !== 'syncing'
  }
  return sync.phase !== 'syncing' && (!baseline || sync.sequence > baseline.sequence)
}

/** Only a refresh requested in this view makes the account show progress. */
export function useManualSync(sync: AccountSyncState | undefined, onRefresh: () => Promise<void>) {
  const [request, setRequest] = useState<Request | null>(null)
  const active = useRef<ActiveRequest | null>(null)
  const refreshing = request !== null && !finished(request, sync)

  // A newer terminal snapshot ends this request even if its IPC response is
  // still pending. Clearing the request also prevents the next automatic round
  // from bringing its indicator back. This is derived state, adjusted on render.
  if (request && !refreshing) setRequest(null)

  useEffect(() => {
    const current = active.current
    if (!current) return
    if (!refreshing) {
      clearTimeout(current.timer)
      active.current = null
    } else if (sync?.phase === 'syncing') {
      // Bound only the wait for a worker to start, never a running sync round.
      clearTimeout(current.timer)
      current.timer = undefined
    }
  }, [refreshing, sync])

  useEffect(() => () => {
    clearTimeout(active.current?.timer)
    active.current = null
  }, [])

  const refresh = useCallback(() => {
    if (active.current || sync?.phase === 'syncing') return
    const next = { baseline: sync }
    const current: ActiveRequest = { timer: undefined }
    const end = () => {
      if (active.current !== current) return false
      clearTimeout(current.timer)
      active.current = null
      setRequest(null)
      return true
    }
    current.timer = setTimeout(end, START_TIMEOUT_MS)
    active.current = current
    setRequest(next)
    // refreshSync acknowledges a wake-up, rather than the completed round.
    // Its resolution must not turn off progress before a terminal event arrives.
    void Promise.resolve().then(onRefresh).catch((error: unknown) => {
      if (end()) toast.error(errorText(error))
    })
  }, [sync, onRefresh])

  return { refreshing, refresh }
}
