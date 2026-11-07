import { useCallback, useEffect, useRef, useState } from 'react'
import { toast } from '@/components/ui/toast'
import { errorText } from '@/i18n/errors'
import type { Account, AccountSyncState } from '@/generated/accounts'

const START_TIMEOUT_MS = 15_000

type Target = { id: string; source?: string; sync: AccountSyncState | undefined }
type Request = { id: string; source?: string; baseline: AccountSyncState | undefined }
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
  return useManualSyncTargets([{ id: 'account', sync }], onRefresh, false)
}

/** Accounts already running join the wait without queuing another sync round. */
export function useManualSyncAll(
  accounts: readonly Pick<Account, 'id' | 'mailboxSourceId'>[],
  syncStates: Record<string, AccountSyncState>,
  onRefresh: (accountId: string) => Promise<void>,
) {
  return useManualSyncTargets(accounts.map((account) => ({
    id: account.id,
    source: account.mailboxSourceId,
    sync: syncStates[account.id],
  })), onRefresh, true)
}

function useManualSyncTargets(targets: Target[], onRefresh: (id: string) => Promise<void>, includeRunning: boolean) {
  const [requests, setRequests] = useState<Request[]>([])
  const active = useRef(new Map<string, ActiveRequest>())
  const pending = requests.filter((request) => {
    const target = targets.find((target) => target.id === request.id)
    return target && target.source === request.source && !finished(request, target.sync)
  })
  const refreshing = pending.length > 0

  // A newer terminal snapshot ends this request even if its IPC response is
  // still pending. Clearing the request also prevents the next automatic round
  // from bringing its indicator back. This is derived state, adjusted on render.
  if (pending.length !== requests.length) setRequests(pending)

  useEffect(() => {
    for (const [id, current] of active.current) {
      if (!pending.some((request) => request.id === id)) {
        clearTimeout(current.timer)
        active.current.delete(id)
      } else if (targets.find((target) => target.id === id)?.sync?.phase === 'syncing') {
        // Bound only the wait for a worker to start, never a running sync round.
        clearTimeout(current.timer)
        current.timer = undefined
      }
    }
  }, [pending, targets])

  useEffect(() => {
    const current = active.current
    return () => {
      for (const request of current.values()) clearTimeout(request.timer)
      current.clear()
    }
  }, [])

  const refresh = useCallback(() => {
    if (active.current.size > 0) return
    const next = targets.filter((target) => includeRunning || target.sync?.phase !== 'syncing')
    setRequests(next.map((target) => ({ id: target.id, source: target.source, baseline: target.sync })))
    for (const target of next) {
      const current: ActiveRequest = { timer: undefined }
      const end = () => {
        if (active.current.get(target.id) !== current) return false
        clearTimeout(current.timer)
        active.current.delete(target.id)
        setRequests((requests) => requests.filter((request) => request.id !== target.id))
        return true
      }
      active.current.set(target.id, current)
      if (target.sync?.phase === 'syncing') continue
      current.timer = setTimeout(end, START_TIMEOUT_MS)
      // refreshSync acknowledges a wake-up, rather than the completed round.
      // Each account settles independently; a rejected request must not hide
      // progress for the other accounts that are still syncing.
      void Promise.resolve().then(() => onRefresh(target.id)).catch((error: unknown) => {
        if (end()) toast.error(errorText(error))
      })
    }
  }, [targets, includeRunning, onRefresh])

  return { refreshing, refresh }
}
