import type { AccountSyncState, FolderSyncState, SyncFailure } from '@/generated/accounts'

/**
 * How long a run of retryable failures may last before the ambient parts of
 * the interface report it. A Wi-Fi switch, a VPN coming up or a short sleep
 * produces a few failed rounds that fix themselves; the worker backs off
 * 1s, 2s, 4s… so half a minute is 5–6 attempts, far past a blip but short
 * enough that a real outage is not hidden for long.
 */
export const QUIET_FOR_MS = 30_000

/**
 * Failures that are weather: they clear on their own, so the interface waits.
 * Everything else — authentication, sign-in, provider policy, missing or
 * untrusted credentials, unsupported servers, storage and protocol errors —
 * needs a person or will not pass on its own, and is shown at once.
 */
const TRANSIENT = new Set<SyncFailure['code']>(['network', 'proxy', 'tls', 'timeout'])

/** Whether the account will probably recover from this failure unaided. */
export function isTransientFailure(failure: SyncFailure): boolean {
  return failure.willRetry && TRANSIENT.has(failure.code)
}

function elapsed(since: string | null, now: number): number | null {
  if (!since) return null
  const started = Date.parse(since)
  return Number.isNaN(started) ? null : now - started
}

/** The oldest `failureSince` among the folders currently failing. */
export function oldestFolderFailure(states: FolderSyncState[]): string | null {
  return states
    .filter((folder) => folder.phase === 'failed' && folder.failureSince)
    .map((folder) => folder.failureSince!)
    .sort((a, b) => Date.parse(a) - Date.parse(b))[0] ?? null
}

/**
 * When the account's current run of failures began: the account's own clock,
 * or else its failing folders', so an account-level round that ended without
 * its own error still knows how long its folders have been failing.
 */
export function failingSince(sync: AccountSyncState | undefined): string | null {
  if (!sync) return null
  const folder = oldestFolderFailure(sync.folders)
  if (!sync.failureSince) return folder
  if (!folder) return sync.failureSince
  return Date.parse(folder) < Date.parse(sync.failureSince) ? folder : sync.failureSince
}

/** Whether a failure is old enough to be worth showing. */
export function failureVisible(failureSince: string | null, now = Date.now()): boolean {
  const age = elapsed(failureSince, now)
  return age !== null && age >= QUIET_FOR_MS
}

/**
 * The failure the ambient interface (the account list, the sync summary, the
 * mail list) should report, or `null` while a retryable one is still young.
 * Settings and the account header use `accountIssues` for the same account:
 * they always say a retry is in progress, because that is where a person
 * goes to look.
 */
export function visibleSyncFailure(sync: AccountSyncState | undefined, now = Date.now()): SyncFailure | null {
  const failure = sync?.failure
  if (!failure) return null
  if (!isTransientFailure(failure)) return failure
  return failureVisible(failingSince(sync), now) ? failure : null
}

/** The same rule for one folder's own state, in folder views. */
export function visibleFolderFailure(folder: FolderSyncState, now = Date.now()): SyncFailure | null {
  if (!folder.failure) return null
  if (!isTransientFailure(folder.failure)) return folder.failure
  return failureVisible(folder.failureSince, now) ? folder.failure : null
}
