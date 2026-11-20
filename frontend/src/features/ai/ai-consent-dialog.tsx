import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import type { AiConsent } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'

/**
 * Asked once per AI connection, before the assistant first reads mail (ai-runtime §5). Agreeing records the
 * consent in the host; the caller starts what it wanted afterwards, in `onGranted`.
 */
export function AiConsentDialog({ client, service, open, onOpenChange, onGranted, onFailed, grant }: {
  client: MailxClient
  /** How consent is recorded; the composer has its own command. Defaults to the assistant's. */
  grant?: () => Promise<AiConsent>
  /** The connection the mail would go to, by name. */
  service: string
  open: boolean
  onOpenChange: (open: boolean) => void
  onGranted: (consent: AiConsent) => void
  onFailed?: (error: unknown) => void
}) {
  useTranslation()
  // Sending mail text out is the careful choice's opposite: the default is to decline.
  const cancel = useRef<HTMLButtonElement>(null)
  const [busy, setBusy] = useState(false)
  const agree = () => {
    if (busy) return
    setBusy(true)
    const record = grant ?? (() => client.assistantGrantConsent())
    record().then(
      (consent) => { setBusy(false); onOpenChange(false); onGranted(consent) },
      (error: unknown) => { setBusy(false); onOpenChange(false); onFailed?.(error) },
    )
  }
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent initialFocus={cancel}>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('ai:consentTitle', { service })}</AlertDialogTitle>
          <AlertDialogDescription>{t('ai:consentBody')}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel ref={cancel}>{t('ai:cancel')}</AlertDialogCancel>
          <AlertDialogAction disabled={busy} onClick={agree}>{t('ai:consentConfirm')}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
