import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import type { MailxClient } from '@/lib/mailx-client'

export function QqAuthHelp({ client }: { client: MailxClient }) {
  useTranslation()
  const [failed, setFailed] = useState(false)
  return (
    <div className="flex flex-col items-start gap-1 text-sm">
      <p className="text-muted-foreground">{t('accounts:qqAuthenticationHelp')}</p>
      <Button variant="link" className="h-auto px-0" onClick={() => {
        setFailed(false)
        void client.openProviderHelp('qq').catch(() => setFailed(true))
      }}>{t('accounts:getAuthorizationCode')}</Button>
      {failed && <p role="alert" className="text-destructive">{t('accounts:helpOpenFailed')}</p>}
    </div>
  )
}
