import { useTranslation } from 'react-i18next'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Switch } from '@/components/ui/switch'
import type { MailxClient } from '@/lib/mailx-client'
import { errorText } from '@/i18n/errors'
import { ShortcutList } from '@/features/mail/shortcut-list'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateGeneral } from './settings-queries'

/** The single-key switch and the full list of shortcuts. */
export function ShortcutsPane({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const settings = useSettings(client)
  const update = useUpdateGeneral(client)
  const enabled = settings.data?.settings.general.singleKeyShortcuts ?? true
  return <SettingsPage>
    {(settings.isError || update.isError) && <Alert variant="destructive">
      <AlertTitle>{t(settings.isError ? 'readFailed' : 'saveFailedTitle')}</AlertTitle>
      <AlertDescription>{errorText(settings.error ?? update.error)}</AlertDescription>
    </Alert>}
    <SettingsGroup title={t('shortcutsSwitchGroup')}>
      <SettingsRow label={t('singleKeyShortcuts')} description={t('singleKeyShortcutsHint')} htmlFor="general.singleKeyShortcuts">
        <Switch id="general.singleKeyShortcuts" checked={enabled}
          disabled={!settings.data || update.isPending}
          onCheckedChange={(singleKeyShortcuts) => update.mutate({ singleKeyShortcuts })} />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title={t('shortcutsList')} description={enabled ? undefined : t('singleKeyShortcutsOff')}>
      <ShortcutList singleKeyOff={!enabled} />
    </SettingsGroup>
  </SettingsPage>
}
