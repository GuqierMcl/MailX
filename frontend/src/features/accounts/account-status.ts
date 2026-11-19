import type { Account, AccountSyncState } from '@/generated/accounts'
import { t } from '@/i18n'
import { providerOf } from './providers'
import { signInNeeded } from './sign-in-state'
import { visibleSyncFailure } from './sync-quiet'

/** Whether a saved credential or the route must be confirmed again before MailX uses it. */
export function needsTrust(account: Account): boolean {
  if (account.authentication.kind === 'ewsNtlm') return account.authentication.credential.trust === 'needsConfirmation' || (account.route.kind === 'custom' && account.route.authentication?.trust === 'needsConfirmation')
  return account.authentication.imap.trust === 'needsConfirmation'
    || account.authentication.smtp.trust === 'needsConfirmation'
    || (account.route.kind === 'custom' && account.route.authentication?.trust === 'needsConfirmation')
}

/** `stopped` needs the user; `retrying` is a failure the worker will try again on its own. */
export type AccountProblem = 'stopped' | 'retrying'

/**
 * The problem the ambient interface should mark. A retryable failure only
 * counts once it has lasted long enough to be more than a blip; a stopped one
 * always does.
 */
export function accountProblem(account: Account, sync: AccountSyncState | undefined): AccountProblem | null {
  if (needsTrust(account) || signInNeeded(account) || (sync?.failure && !sync.failure.willRetry)) return 'stopped'
  return visibleSyncFailure(sync) ? 'retrying' : null
}

/** What keeps this account from syncing, as the page needs to present it. */
export function accountIssues(account: Account, sync: AccountSyncState | undefined) {
  const trust = needsTrust(account)
  // A failure that will be retried is transient; only a stopped sync needs the user.
  const stopped = sync?.failure && !sync.failure.willRetry ? sync.failure : null
  // A browser sign-in has no connection settings to edit: renewing the sign-in is the only repair.
  const oauth = account.authentication.kind === 'oauth2'
  const stoppedForSignIn = oauth && (stopped?.code === 'signInRequired' || stopped?.code === 'credentialMissing')
  // Only a sign-in the account can't sync without is an issue; otherwise it is a connection action.
  const signInFirst = oauth && signInNeeded(account, stoppedForSignIn)
  // The header says a retry is in progress even while the rest of the interface stays
  // quiet: a person who opened the settings is asking what is going on.
  const retrying = sync?.failure && sync.failure.willRetry ? sync.failure : null
  return { needsTrust: trust, stopped, oauth, stoppedForSignIn, signInFirst, retrying }
}

/** How the account connects: the plain protocol pair, or the provider's browser sign-in. */
export function connectionMethod(account: Account): string {
  if ('ews' in account) return t('accounts:ewsConnection')
  return account.authentication.kind === 'oauth2'
    ? t('accounts:browserSignInMethod', { name: providerOf(account.providerId).name })
    : t('accounts:methodImapSmtp')
}

/** A credential problem to show beside the connection method, so it is never hidden in the fold. */
export function credentialProblem(account: Account): string | null {
  if (needsTrust(account)) return t('accounts:credentialsNeedConfirmationShort')
  if (account.authentication.kind === 'ewsNtlm' && !account.authentication.credential.passwordConfigured) return t('accounts:missingPassword')
  if (account.authentication.kind === 'password' && !(account.authentication.imap.passwordConfigured && account.authentication.smtp.passwordConfigured)) return t('accounts:missingPassword')
  return null
}
