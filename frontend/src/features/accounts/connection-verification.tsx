import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Account, AccountConnectionVerification } from '@/generated/accounts'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { QqAuthHelp } from './qq-auth-help'

type VerificationState =
  | { kind: 'idle' | 'checking' | 'cancelled' }
  | { kind: 'ready'; report: AccountConnectionVerification }
  | { kind: 'failed'; failure: ClientFailure }

function failureText(code: string | null): string {
  if (code === 'authUnavailable') return t('accounts:verificationAuthUnavailable')
  if (code === 'configuration') return t('accounts:verificationConfiguration')
  if (code === 'secretStore') return t('accounts:verificationSecretStore')
  if (code === 'cancelled') return t('accounts:verificationCancelled')
  return errorText({ code })
}

/** A report is scoped to this mounted account revision, never a persistent promise of delivery. */
export function ConnectionVerification({ account, client }: { account: Account; client: MailxClient }) {
  return <ConnectionVerificationAttempt key={`${account.id}:${account.revision}`} account={account} client={client} />
}

function ConnectionVerificationAttempt({ account, client }: { account: Account; client: MailxClient }) {
  useTranslation()
  const [state, setState] = useState<VerificationState>({ kind: 'idle' })
  const pending = useRef<AbortController | null>(null)
  useEffect(() => () => { pending.current?.abort(); pending.current = null }, [])

  async function verify() {
    if (pending.current) return
    const controller = new AbortController()
    pending.current = controller
    setState({ kind: 'checking' })
    try {
      const report = await client.verifyAccountConnection(account.id, account.revision, controller.signal)
      if (pending.current !== controller || controller.signal.aborted) return
      if (report.accountId !== account.id || report.revision !== account.revision) {
        setState({ kind: 'failed', failure: { code: 'conflict', field: null, message: '' } })
      } else setState({ kind: 'ready', report })
    } catch (cause) {
      if (pending.current === controller && !controller.signal.aborted) setState({ kind: 'failed', failure: clientFailure(cause) })
    } finally {
      if (pending.current === controller) pending.current = null
    }
  }

  const report = state.kind === 'ready' ? state.report : null
  const passed = report?.imap.status === 'verified' && report.smtp.status === 'verified'
  const authenticationFailed = report?.imap.code === 'authentication' || report?.smtp.code === 'authentication'

  return (
    <div className="flex flex-col items-start gap-3">
      <p className="text-xs text-muted-foreground">{t('accounts:verificationHint')}</p>
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" disabled={!client.available || state.kind === 'checking'} onClick={() => void verify()}>
          {t(state.kind === 'checking' ? 'accounts:verifyingConnection' : 'accounts:verifyConnection')}
        </Button>
        {state.kind === 'checking' && <Button variant="ghost" onClick={() => {
          pending.current?.abort()
          pending.current = null
          setState({ kind: 'cancelled' })
        }}>{t('accounts:cancelVerification')}</Button>}
      </div>
      <div role="status" aria-live="polite" className="w-full text-sm">
        {state.kind === 'cancelled' && <p className="text-muted-foreground">{t('accounts:verificationCancelled')}</p>}
        {state.kind === 'failed' && <p className="text-destructive">{state.failure.code === 'conflict' ? t('accounts:verificationStale') : errorText(state.failure)}</p>}
        {report && (
          <div className="flex flex-col gap-2">
            <p className={passed ? 'text-muted-foreground' : 'text-destructive'}>{t(passed ? 'accounts:verificationSucceeded' : 'accounts:verificationFailed')}</p>
            {(['imap', 'smtp'] as const).map((protocol) => (
              <div key={protocol} className="flex flex-wrap justify-between gap-x-4 gap-y-1">
                <span>{t(protocol === 'imap' ? 'accounts:verificationIncoming' : 'accounts:verificationOutgoing')}</span>
                <span className={report[protocol].status === 'verified' ? 'text-muted-foreground' : 'text-destructive'}>
                  {report[protocol].status === 'verified' ? t('accounts:verificationPassed') : failureText(report[protocol].code)}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
      {account.providerId === 'qq' && authenticationFailed && <QqAuthHelp client={client} />}
    </div>
  )
}
