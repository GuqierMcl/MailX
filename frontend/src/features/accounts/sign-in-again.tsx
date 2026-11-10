import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Account } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'

type SignInAgainProps = {
  client: MailxClient
  /** An account that signs in through the browser. */
  account: Account
  disabled?: boolean
  /** Shows the prominent warning even when the account itself still reads as connected, e.g. after a stopped sync. */
  required?: boolean
  onSignedIn: (account: Account) => void
}

/**
 * Renews the browser sign-in of an existing account. When the account needs it,
 * the block is a prominent warning; otherwise it stays a quiet button so a
 * working sign-in can still be refreshed on purpose.
 */
export function SignInAgain({ client, account, disabled = false, required = false, onSignedIn }: SignInAgainProps) {
  useTranslation()
  const needed = required || (account.authentication.kind === 'oauth2' && account.authentication.state === 'needsReauthorization')
  const [waiting, setWaiting] = useState(false)
  const [failure, setFailure] = useState<ClientFailure | null>(null)
  const session = useRef<{ id: string | null; canceled: boolean } | null>(null)
  const alive = useRef(true)

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      // Leaving the account abandons a sign-in that is still waiting.
      const current = session.current
      if (current) {
        current.canceled = true
        if (current.id) void client.cancelOAuthAccount(current.id).catch(() => undefined)
      }
    }
  }, [client])

  if (account.authentication.kind !== 'oauth2' || account.providerId === null) return null
  const providerId = account.providerId

  async function start() {
    if (waiting) return
    const mine = { id: null as string | null, canceled: false }
    session.current = mine
    setFailure(null)
    setWaiting(true)
    try {
      const handle = await client.reauthorizeOAuthAccount({ providerId, accountId: account.id, expectedRevision: account.revision })
      mine.id = handle.sessionId
      if (mine.canceled) await client.cancelOAuthAccount(handle.sessionId).catch(() => undefined)
      const renewed = await client.finishOAuthAccount(handle.sessionId)
      if (alive.current && !mine.canceled) onSignedIn(renewed)
    } catch (cause) {
      if (alive.current && !mine.canceled) setFailure(clientFailure(cause))
    } finally {
      if (session.current === mine) session.current = null
      if (alive.current) setWaiting(false)
    }
  }

  function cancel() {
    const current = session.current
    if (!current) return
    current.canceled = true
    if (current.id) void client.cancelOAuthAccount(current.id).catch(() => undefined)
    setWaiting(false)
  }

  const actions = waiting
    ? <Button variant="outline" className="mt-3 w-fit" onClick={cancel}>{t('accounts:cancelSignIn')}</Button>
    : <Button variant={needed ? 'default' : 'outline'} className={needed ? 'mt-3 w-fit' : 'w-fit'} disabled={disabled || !client.available} onClick={() => void start()}>
      {t('accounts:signInAgain')}
    </Button>

  return (
    <div className="flex flex-col gap-3">
      {needed ? (
        <Alert variant="destructive">
          <AlertTitle>{t('accounts:signInAgainNeededTitle')}</AlertTitle>
          <AlertDescription>{t('accounts:signInAgainNeededDescription')}</AlertDescription>
          {actions}
        </Alert>
      ) : actions}
      {waiting && (
        <Alert role="status">
          <AlertTitle>{t('accounts:signInWaitingTitle')}</AlertTitle>
          <AlertDescription>{t('accounts:signInWaitingDescription')}</AlertDescription>
        </Alert>
      )}
      {failure && (
        <Alert variant="destructive">
          <AlertTitle>{t('accounts:signInAgainFailed')}</AlertTitle>
          <AlertDescription>{errorText(failure)}</AlertDescription>
        </Alert>
      )}
    </div>
  )
}
