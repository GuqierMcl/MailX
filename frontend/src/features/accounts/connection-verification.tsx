import { useTranslation } from 'react-i18next'
import type { AccountConnectionVerification } from '@/generated/accounts'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import type { MailxClient } from '@/lib/mailx-client'
import { ProviderAuthHelp } from './provider-auth-help'
import { failureText, verificationReport, type ConnectionVerificationHandle, type VerificationState } from './use-connection-verification'

type Protocol = 'imap' | 'smtp'
type ProtocolResult = Pick<AccountConnectionVerification, 'imap' | 'smtp'>[Protocol]

/** The check and cancel buttons; `onStart` runs before a new check begins. */
export function ConnectionVerificationActions({ verification, client, onStart }: {
  verification: ConnectionVerificationHandle
  client: MailxClient
  onStart?: () => void
}) {
  useTranslation()
  return (
    <>
      {verification.checking && <Button variant="ghost" onClick={verification.cancel}>{t('accounts:cancelVerification')}</Button>}
      <Button variant="outline" disabled={!client.available || verification.checking} onClick={() => {
        onStart?.()
        void verification.verify()
      }}>
        {t(verification.checking ? 'accounts:verifyingConnection' : 'accounts:verifyConnection')}
      </Button>
    </>
  )
}

/** A short result to append to the connection method: passed, or which side did not. */
export function VerificationSummary({ state }: { state: VerificationState }) {
  useTranslation()
  const report = verificationReport(state)
  if (!report) return null
  const incoming = report.imap.status === 'verified'
  const outgoing = report.smtp.status === 'verified'
  if (incoming && outgoing) return <span className="text-xs text-muted-foreground">· {t('accounts:verificationBothPassed')}</span>
  const key = incoming ? 'accounts:verificationOutgoingFailed' : outgoing ? 'accounts:verificationIncomingFailed' : 'accounts:verificationBothFailed'
  return <span className="text-xs text-destructive">· {t(key)}</span>
}

/** A request that gave no report at all: cancelled, or failed as a whole. Empty when there is nothing to say. */
export function VerificationNotice({ state }: { state: VerificationState }) {
  useTranslation()
  return (
    <div role="status" aria-live="polite" className="text-xs empty:hidden">
      {state.kind === 'cancelled' && <p className="text-muted-foreground">{t('accounts:verificationCancelled')}</p>}
      {state.kind === 'failed' && <p className="text-destructive">{state.failure.code === 'conflict' ? t('accounts:verificationStale') : errorText(state.failure)}</p>}
    </div>
  )
}

/** One protocol's outcome under its server row: a quiet pass, or the reason it failed. */
export function ProtocolOutcome({ state, protocol }: { state: VerificationState; protocol: Protocol }) {
  useTranslation()
  const report = verificationReport(state)
  if (!report) return null
  const result = report[protocol]
  return result.status === 'verified'
    ? <p className="text-xs text-muted-foreground">{t('accounts:verificationPassed')}</p>
    : <p className="text-xs text-destructive">{failureText(result.code)}</p>
}

/** Reviewed provider guidance for a rejected login or a provider refusal; nothing otherwise. */
export function VerificationHelp({ report, client, providerId }: {
  report: Pick<AccountConnectionVerification, 'imap' | 'smtp'> | null
  client: MailxClient
  providerId: string | null
}) {
  if (!report) return null
  const authenticationFailed = report.imap.code === 'authentication' || report.smtp.code === 'authentication'
  const policyFailed = report.imap.code === 'providerPolicy' || report.smtp.code === 'providerPolicy'
  if (!authenticationFailed && !policyFailed) return null
  return <ProviderAuthHelp client={client} providerId={providerId} reason={policyFailed ? 'providerPolicy' : 'authentication'} />
}

/** Saved and unsaved connections share the same protocol results and reviewed help. */
export function ConnectionVerificationReport({ report, client, providerId }: {
  report: Pick<AccountConnectionVerification, 'imap' | 'smtp'>
  client: MailxClient
  providerId: string | null
}) {
  useTranslation()
  const passed = report.imap.status === 'verified' && report.smtp.status === 'verified'
  return <div className="flex w-full flex-col gap-3">
    {passed && <p className="text-xs text-muted-foreground">{t('accounts:verificationSucceeded')}</p>}
    <div className="flex flex-col gap-2">
      {(['imap', 'smtp'] as const).map((protocol) => <ReportRow key={protocol} result={report[protocol]}
        label={t(protocol === 'imap' ? 'accounts:verificationIncoming' : 'accounts:verificationOutgoing')} />)}
    </div>
    <VerificationHelp report={report} client={client} providerId={providerId} />
  </div>
}

function ReportRow({ label, result }: { label: string; result: ProtocolResult }) {
  const ok = result.status === 'verified'
  return (
    <div className="flex flex-col gap-0.5">
      <div className="flex items-baseline justify-between gap-4 text-sm">
        <span>{label}</span>
        <span className={ok ? 'text-xs text-muted-foreground' : 'text-xs text-destructive'}>{t(ok ? 'accounts:verificationPassed' : 'accounts:verificationNotPassed')}</span>
      </div>
      {!ok && <p className="text-xs text-destructive">{failureText(result.code)}</p>}
    </div>
  )
}
