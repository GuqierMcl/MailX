import { useTranslation } from 'react-i18next'
import type { SyncWindow } from '@/generated/accounts'
import { ErrorNotice } from '@/components/error-notice'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import type { MailxClient } from '@/lib/mailx-client'
import { errorText } from '@/i18n/errors'
import { SettingsGroup, SettingsPage, SettingsRow, SettingsSaveError } from './settings-layout'
import { useSettings, useUpdateMailSettings } from './settings-queries'

const SYNC_WINDOWS: SyncWindow[] = ['days30', 'days90', 'year1', 'all']

export function MailEnhancementsPane({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const settings = useSettings(client)
  const update = useUpdateMailSettings(client)
  const syncWindow = settings.data?.settings.mail.syncWindow ?? 'days30'
  const syncWindowItems: Record<SyncWindow, string> = Object.fromEntries(
    SYNC_WINDOWS.map((option) => [option, t(`syncWindowOption.${option}`)]),
  ) as Record<SyncWindow, string>
  return <SettingsPage>
    <SettingsGroup title={t('syncHistory')}>
      <SettingsRow label={t('syncWindow')} description={t('syncWindowDescription')} htmlFor="mail.syncWindow">
        <Select value={syncWindow} items={syncWindowItems} disabled={!settings.data || update.isPending}
          onValueChange={(value) => { if (value) update.mutate({ syncWindow: value as SyncWindow }) }}>
          <SelectTrigger id="mail.syncWindow" aria-label={t('syncWindow')} className="w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SYNC_WINDOWS.map((option) => (
              <SelectItem key={option} value={option}>{syncWindowItems[option]}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title={t('actionItemsDetection')}>
      <SettingsRow label={t('detectActionItems')} description={t('detectActionItemsHint')} htmlFor="mail.detectActionItems">
        <Switch id="mail.detectActionItems" checked={settings.data?.settings.mail.detectActionItems ?? true}
          disabled={!settings.data || update.isPending}
          onCheckedChange={(detectActionItems) => update.mutate({ detectActionItems })} />
      </SettingsRow>
    </SettingsGroup>
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
