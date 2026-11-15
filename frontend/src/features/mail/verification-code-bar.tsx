import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import { readyVerificationCode, type VerificationCodes } from './use-verification-codes'

export function VerificationCodeBar({ emailId, verification }: { emailId: string; verification?: VerificationCodes }) {
  useTranslation()
  const ready = readyVerificationCode(verification?.status(emailId))
  if (!ready || !verification) return null
  const busy = verification.busy(emailId)
  const copied = verification.copied(emailId, ready.recognitionId)
  return <div role="status" aria-label={t('mail:verificationCodeBar')} className="flex h-9 w-full shrink-0 items-center gap-3 rounded-md bg-muted px-3 text-xs">
    <span className="flex min-w-0 flex-1 items-baseline gap-2 leading-5">
      <span className="shrink-0 leading-5">{t('mail:verificationCodeLabel')}</span>
      <strong className="truncate font-mono text-sm leading-5 tracking-wide">{ready.code}</strong>
    </span>
    <Button size="sm" variant="outline" className="h-6 shrink-0 text-xs" disabled={busy}
      onClick={() => void verification.copy(emailId, ready.recognitionId)}>
      {busy ? t('mail:verificationCopying') : copied ? t('mail:verificationCopiedShort') : t('mail:verificationCopy')}
    </Button>
  </div>
}
