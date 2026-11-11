import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import type { EmailTranslation } from './use-email-translation'

/** Asked once per translation service, before the first message is sent to it. */
export function TranslationConsentDialog({ translation }: { translation: EmailTranslation }) {
  useTranslation()
  const service = translation.consentService
  // Sending mail text out is the careful choice's opposite: the default is to decline.
  const cancel = useRef<HTMLButtonElement>(null)
  return (
    <AlertDialog open={service !== null} onOpenChange={(open) => { if (!open) translation.cancelConsent() }}>
      <AlertDialogContent initialFocus={cancel}>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('mail:translationConsentTitle', { service: service ?? '' })}</AlertDialogTitle>
          <AlertDialogDescription>{t('mail:translationConsentBody', { service: service ?? '' })}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel ref={cancel}>{t('mail:cancel')}</AlertDialogCancel>
          <AlertDialogAction onClick={() => { void translation.confirmConsent() }}>{t('mail:translationConsentConfirm')}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
