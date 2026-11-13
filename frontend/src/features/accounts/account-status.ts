import type { Account, AccountSyncState } from '@/generated/accounts'
import { t } from '@/i18n'
import { providerOf } from './providers'
import { signInNeeded } from './sign-in-state'

/** Whether a saved credential or the route must be confirmed again before MailX uses it. */
export function needsTrust(account: Account): boolean {
  return account.authentication.imap.trust === 'needsConfirmation'
    || account.authentication.smtp.trust === 'needsConfirmation'
    || (account.route.kind === 'custom' && account.route.authentication?.trust === 'needsConfirmation')
}

/** `stopped` needs the user; `retrying` is a failure the worker will try again on its own. */
export type AccountProblem = 'stopped' | 'retrying'

export function accountProblem(account: Account, sync: AccountSyncState | undefined): AccountProblem | null {
  if (needsTrust(account) || signInNeeded(account) || (sync?.failure && !sync.failure.willRetry)) return 'stopped'
  return sync?.failure ? 'retrying' : null
}

/** The first account in the given order that has a problem, a stopped one before one that is retrying. */
export function firstProblemAccountId(accounts: Account[], syncStates: Record<string, AccountSyncState>): string | null {
  const problems = accounts.map((account) => ({ id: account.id, problem: accountProblem(account, syncStates[account.id]) }))
  return (problems.find((entry) => entry.problem === 'stopped') ?? problems.find((entry) => entry.problem === 'retrying'))?.id ?? null
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
  return { needsTrust: trust, stopped, oauth, stoppedForSignIn, signInFirst, retrying: sync?.failure && sync.failure.willRetry ? sync.failure : null }
}

/** How the account connects: the plain protocol pair, or the provider's browser sign-in. */
export function connectionMethod(account: Account): string {
  return account.authentication.kind === 'oauth2'
    ? t('accounts:browserSignInMethod', { name: providerOf(account.providerId).name })
    : t('accounts:methodImapSmtp')
}

/** A credential problem to show beside the connection method, so it is never hidden in the fold. */
export function credentialProblem(account: Account): string | null {
  if (needsTrust(account)) return t('accounts:credentialsNeedConfirmationShort')
  if (account.authentication.kind === 'password' && !(account.authentication.imap.passwordConfigured && account.authentication.smtp.passwordConfigured)) return t('accounts:missingPassword')
  return null
}
