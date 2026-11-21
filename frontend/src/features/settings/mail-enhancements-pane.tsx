import { useTranslation } from 'react-i18next'
import { ErrorNotice } from '@/components/error-notice'
import { Switch } from '@/components/ui/switch'
import type { MailxClient } from '@/lib/mailx-client'
import { errorText } from '@/i18n/errors'
import { SettingsGroup, SettingsPage, SettingsRow, SettingsSaveError } from './settings-layout'
import { useSettings, useUpdateMailSettings } from './settings-queries'

export function MailEnhancementsPane({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const settings = useSettings(client)
  const update = useUpdateMailSettings(client)
  return <SettingsPage>
    <SettingsGroup title={t('verificationCodes')}>
      <SettingsRow label={t('trashVerificationMailAfterCopy')} description={t('trashVerificationMailAfterCopyHint')} htmlFor="mail.trashVerificationMailAfterCopy">
        <Switch id="mail.trashVerificationMailAfterCopy" checked={settings.data?.settings.mail.trashVerificationMailAfterCopy ?? false}
          disabled={!settings.data || update.isPending}
          onCheckedChange={(trashVerificationMailAfterCopy) => update.mutate({ trashVerificationMailAfterCopy })} />
      </SettingsRow>
      {settings.isError && <ErrorNotice message={t('readFailed')} detail={errorText(settings.error)} actionLabel={t('retry')}
        onAction={() => void settings.refetch()} actionDisabled={settings.isFetching} />}
      <SettingsSaveError update={update} />
    </SettingsGroup>
  </SettingsPage>
}
