import { useTranslation } from 'react-i18next'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { LanguagePreference } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { errorText } from '@/i18n/errors'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateGeneral } from './settings-queries'

export function GeneralPane({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const settings = useSettings(client)
  const update = useUpdateGeneral(client)
  const items = { system: t('system'), 'zh-CN': '简体中文', en: 'English' }
  return <SettingsPage>
    {(settings.isError || update.isError) && <Alert variant="destructive">
      <AlertTitle>{t(settings.isError ? 'readFailed' : 'saveFailedTitle')}</AlertTitle>
      <AlertDescription>{errorText(settings.error ?? update.error)}</AlertDescription>
    </Alert>}
    <SettingsGroup title={t('general')}>
      <SettingsRow label={t('language')} description={t('languageHint')} htmlFor="general.language">
        <Select value={settings.data?.settings.general.language ?? 'system'} items={items}
          disabled={!settings.data || update.isPending}
          onValueChange={(language: LanguagePreference | null) => { if (language) update.mutate({ language }) }}>
          <SelectTrigger id="general.language" className="w-40"><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(items).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
}
