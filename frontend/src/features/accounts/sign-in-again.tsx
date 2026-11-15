import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Account } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { toast } from '@/components/ui/toast'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { signInNeeded } from './sign-in-state'

type SignInAgainProps = {
  client: MailxClient
  /** An account that signs in through the browser. */
  account: Account
  disabled?: boolean
  /** Makes this the primary action even when the account itself still reads as connected, e.g. after a stopped sync. */
  required?: boolean
  onSignedIn: (account: Account) => void
}

/**
 * The button that renews the browser sign-in of an existing account, for the
 * page's action bar. It is the primary action when the account needs it and a
 * quiet one otherwise, so a working sign-in can still be refreshed on purpose.
 * Waiting, success and failure are events, reported as toasts; what the
 * account needs stays in the page header.
 */
export function SignInAgain({ client, account, disabled = false, required = false, onSignedIn }: SignInAgainProps) {
  useTranslation()
  const needed = signInNeeded(account, required)
  const [waiting, setWaiting] = useState(false)
  const session = useRef<{ id: string | null; canceled: boolean } | null>(null)
  const alive = useRef(true)
  const waitingToast = useRef<string | null>(null)
  const closeWaiting = () => {
    if (waitingToast.current) toast.close(waitingToast.current)
    waitingToast.current = null
  }

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      if (waitingToast.current) toast.close(waitingToast.current)
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
    setWaiting(true)
    const toastId = toast.info(t('accounts:signInWaitingTitle'), { description: t('accounts:signInWaitingDescription'), timeout: 0 })
    waitingToast.current = toastId
    try {
      const handle = await client.reauthorizeOAuthAccount({ providerId, accountId: account.id, expectedRevision: account.revision })
      mine.id = handle.sessionId
      if (mine.canceled) await client.cancelOAuthAccount(handle.sessionId).catch(() => undefined)
      const renewed = await client.finishOAuthReauthorization(handle.sessionId)
      if (alive.current && !mine.canceled) {
        onSignedIn(renewed)
        toast.success(t('accounts:signedInAgain'))
      }
    } catch (cause) {
      if (alive.current && !mine.canceled) toast.error(t('accounts:signInAgainFailed'), { description: errorText(clientFailure(cause)) })
    } finally {
      if (session.current === mine) session.current = null
      toast.close(toastId)
      if (waitingToast.current === toastId) waitingToast.current = null
      if (alive.current) setWaiting(false)
    }
  }

  function cancel() {
    const current = session.current
    if (!current) return
    current.canceled = true
    if (current.id) void client.cancelOAuthAccount(current.id).catch(() => undefined)
    closeWaiting()
    setWaiting(false)
  }

  return waiting
    ? <Button variant="outline" onClick={cancel}>{t('accounts:cancelSignIn')}</Button>
    : <Button variant={needed ? 'default' : 'outline'} disabled={disabled || !client.available} onClick={() => void start()}>
      {t('accounts:signInAgain')}
    </Button>
}
