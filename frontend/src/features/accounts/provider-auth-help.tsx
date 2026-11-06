import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import type { MailxClient } from '@/lib/mailx-client'
import { providerCredentialText, providerOf } from './providers'

export function ProviderAuthHelp({ client, providerId, reason = 'authentication' }: {
  client: MailxClient
  providerId: string | null
  reason?: 'authentication' | 'providerPolicy'
}) {
  useTranslation()
  const [failed, setFailed] = useState(false)
  const setup = providerOf(providerId).setup
  const message = reason === 'providerPolicy' ? setup?.providerPolicyHelp : setup?.authenticationHelp
  if (!providerId || !message) return null
  return (
    <div className="flex flex-col items-start gap-1 text-sm">
      <p className="text-muted-foreground">{message}</p>
      <Button variant="link" className="h-auto px-0" onClick={() => {
        setFailed(false)
        void client.openProviderHelp(providerId).catch(() => setFailed(true))
      }}>{reason === 'providerPolicy' ? t('accounts:providerPolicyHelpLink') : providerCredentialText(setup).help}</Button>
      {failed && <p role="alert" className="text-destructive">{t('accounts:helpOpenFailed')}</p>}
    </div>
  )
}
