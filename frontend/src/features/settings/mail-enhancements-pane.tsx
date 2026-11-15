import { useTranslation } from 'react-i18next'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Switch } from '@/components/ui/switch'
import type { MailxClient } from '@/lib/mailx-client'
import { errorText } from '@/i18n/errors'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateMailSettings } from './settings-queries'

export function MailEnhancementsPane({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const settings = useSettings(client)
  const update = useUpdateMailSettings(client)
  return <SettingsPage>
    {(settings.isError || update.isError) && <Alert variant="destructive">
      <AlertTitle>{t(settings.isError ? 'readFailed' : 'saveFailedTitle')}</AlertTitle>
      <AlertDescription>{errorText(settings.error ?? update.error)}</AlertDescription>
    </Alert>}
    <SettingsGroup title={t('verificationCodes')}>
      <SettingsRow label={t('trashVerificationMailAfterCopy')} description={t('trashVerificationMailAfterCopyHint')} htmlFor="mail.trashVerificationMailAfterCopy">
        <Switch id="mail.trashVerificationMailAfterCopy" checked={settings.data?.settings.mail.trashVerificationMailAfterCopy ?? false}
          disabled={!settings.data || update.isPending}
          onCheckedChange={(trashVerificationMailAfterCopy) => update.mutate({ trashVerificationMailAfterCopy })} />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
}
