import { useEffect, useRef, useState } from 'react'
import type { Account, AccountConnectionVerification } from '@/generated/accounts'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'

export type VerificationState =
  | { kind: 'idle' | 'checking' | 'cancelled' }
  | { kind: 'ready'; report: AccountConnectionVerification }
  | { kind: 'failed'; failure: ClientFailure }

const IDLE: VerificationState = { kind: 'idle' }

export function failureText(code: string | null): string {
  if (code === 'authUnavailable') return t('accounts:verificationAuthUnavailable')
  if (code === 'configuration') return t('accounts:verificationConfiguration')
  if (code === 'secretStore') return t('accounts:verificationSecretStore')
  if (code === 'cancelled') return t('accounts:verificationCancelled')
  return errorText({ code })
}

export type ConnectionVerificationHandle = {
  state: VerificationState
  checking: boolean
  verify: () => Promise<void>
  cancel: () => void
}

/**
 * Verifies the saved connection of one account. A result belongs to this
 * account revision only and is never a promise of delivery: it is dropped when
 * the revision changes, and a request still running then is aborted, as it is
 * when the owner goes away. The buttons and the results can live in different
 * places because both read this one handle.
 */
export function useConnectionVerification(account: Account, client: MailxClient): ConnectionVerificationHandle {
  const scope = `${account.id}:${account.revision}`
  const [stored, setStored] = useState<{ scope: string; state: VerificationState }>({ scope, state: IDLE })
  const state = stored.scope === scope ? stored.state : IDLE
  const pending = useRef<AbortController | null>(null)
  useEffect(() => () => { pending.current?.abort(); pending.current = null }, [scope])

  async function verify() {
    if (pending.current) return
    const controller = new AbortController()
    pending.current = controller
    setStored({ scope, state: { kind: 'checking' } })
    try {
      const report = await client.verifyAccountConnection(account.id, account.revision, controller.signal)
      if (pending.current !== controller || controller.signal.aborted) return
      if (report.accountId !== account.id || report.revision !== account.revision) {
        setStored({ scope, state: { kind: 'failed', failure: { code: 'conflict', field: null, message: '' } } })
      } else setStored({ scope, state: { kind: 'ready', report } })
    } catch (cause) {
      if (pending.current === controller && !controller.signal.aborted) setStored({ scope, state: { kind: 'failed', failure: clientFailure(cause) } })
    } finally {
      if (pending.current === controller) pending.current = null
    }
  }

  function cancel() {
    pending.current?.abort()
    pending.current = null
    setStored({ scope, state: { kind: 'cancelled' } })
  }

  return { state, checking: state.kind === 'checking', verify, cancel }
}

export function verificationReport(state: VerificationState): AccountConnectionVerification | null {
  return state.kind === 'ready' ? state.report : null
}

/** Whether either protocol was checked and did not pass. */
export function verificationFailed(state: VerificationState): boolean {
  const report = verificationReport(state)
  return !!report && (report.imap.status !== 'verified' || report.smtp.status !== 'verified')
}
