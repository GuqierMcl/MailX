import type { Account, AccountSyncState, Folder } from '@/generated/accounts'
import type { ListSyncStatus } from './email-list'
import { errorText } from '@/i18n/errors'

/** The snapshot query and event delivery may complete in either order. */
export function acceptsSyncState(account: Pick<Account, 'mailboxSourceId'>, previous: AccountSyncState | undefined, next: AccountSyncState) {
  if (next.mailboxSourceId !== account.mailboxSourceId) return false
  if (!previous || previous.mailboxSourceId !== next.mailboxSourceId) return true
  return next.generation > previous.generation ||
    (next.generation === previous.generation && next.sequence > previous.sequence)
}

export function folderSyncStatus(folders: Folder[], states: Record<string, AccountSyncState>, windowCapped: boolean): ListSyncStatus {
  if (folders.length === 0) {
    const accounts = Object.values(states)
    const failure = accounts.find((state) => state.phase === 'failed')?.failure
    const phase = failure ? 'failed' : accounts.some((state) => state.phase === 'syncing') ? 'syncing'
      : accounts.length > 0 && accounts.every((state) => state.phase === 'idle') ? 'idle' : 'waiting'
    return { phase, failure: failure ? errorText(failure) : null, windowCapped }
  }
  const scoped = folders.map((folder) => {
    const account = states[folder.accountId]
    const state = account?.folders.find((item) => item.folderId === folder.id)
    // Account authentication/route failures also explain folders that could not start.
    return account?.phase === 'failed' && state?.phase !== 'failed' && state?.phase !== 'idle'
      ? { phase: 'failed' as const, failure: account.failure }
      : state ?? { phase: 'waiting' as const, failure: null }
  })
  const failed = scoped.find((state) => state.phase === 'failed')
  const phase = failed ? 'failed' : scoped.some((state) => state.phase === 'syncing') ? 'syncing'
    : scoped.length === 0 || scoped.some((state) => state.phase === 'waiting') ? 'waiting' : 'idle'
  return { phase, failure: failed?.failure ? errorText(failed.failure) : null, windowCapped }
}
