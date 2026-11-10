import type { Account } from '@/generated/accounts'

/** Whether the account can't sync until its browser sign-in is renewed. */
export function signInNeeded(account: Account, required = false) {
  return required || (account.authentication.kind === 'oauth2' && account.authentication.state === 'needsReauthorization')
}
