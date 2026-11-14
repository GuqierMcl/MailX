import type { Account, AccountSyncState } from '@/generated/accounts'
import { accountProblem } from '@/features/accounts/account-status'

/**
 * The overview's `dayStarts`: the local start of each of the last fourteen
 * days, oldest first, the last one the start of today. Built from the local
 * calendar date, so a day with a daylight-saving change is 23 or 25 hours
 * long instead of shifting every start by an hour.
 */
export function dayStarts(now: Date): string[] {
  return Array.from({ length: 14 }, (_, index) =>
    new Date(now.getFullYear(), now.getMonth(), now.getDate() - 13 + index).toISOString())
}

export type WeekComparison =
  | { kind: 'only' }
  | { kind: 'same' }
  | { kind: 'more' | 'fewer'; diff: number }

/**
 * How the last seven days compare with the seven before: only when the cache
 * holds both weeks, and a small change (two messages or a tenth) counts as the same.
 */
export function compareWeeks(last: number, previous: number, complete: boolean): WeekComparison {
  if (!complete) return { kind: 'only' }
  const diff = last - previous
  if (Math.abs(diff) <= Math.max(2, Math.round(previous * 0.1))) return { kind: 'same' }
  return diff > 0 ? { kind: 'more', diff } : { kind: 'fewer', diff: -diff }
}

export type SyncSummary =
  | { kind: 'synced'; at: string }
  | { kind: 'syncing' }
  | { kind: 'issue' }

/**
 * One line for every account: a problem anywhere wins, then a round in
 * progress (or an account that has not finished one yet), else the oldest
 * last successful sync, so "synced 2 minutes ago" holds for every mailbox.
 * `null` with no accounts or before any account reported its state.
 */
export function syncSummary(accounts: Account[], syncStates: Record<string, AccountSyncState>): SyncSummary | null {
  if (accounts.length === 0) return null
  if (accounts.some((account) => accountProblem(account, syncStates[account.id]) !== null)) return { kind: 'issue' }
  const states = accounts.map((account) => syncStates[account.id])
  if (states.some((state) => state?.phase === 'syncing')) return { kind: 'syncing' }
  if (states.some((state) => state === undefined)) return null
  const times = states.map((state) => state && lastSynced(state))
  if (times.some((time) => time == null)) return { kind: 'syncing' }
  const oldest = Math.min(...times.map((time) => Date.parse(time!)))
  return { kind: 'synced', at: new Date(oldest).toISOString() }
}

/** The account's last full round, or else its folders' oldest one. */
function lastSynced(state: AccountSyncState): string | null {
  if (state.lastSyncedAt) return state.lastSyncedAt
  const folders = state.folders.map((folder) => folder.lastSyncedAt)
  if (folders.length === 0 || folders.some((time) => !time)) return null
  return folders.reduce((oldest, time) => (Date.parse(time!) < Date.parse(oldest!) ? time : oldest))
}
