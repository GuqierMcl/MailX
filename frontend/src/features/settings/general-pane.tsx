import { useTranslation } from 'react-i18next'
import { useState } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import type { CloseBehavior, LanguagePreference } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { errorText } from '@/i18n/errors'
import { useLifecycle } from '@/features/lifecycle/lifecycle-context'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateGeneral } from './settings-queries'

export function GeneralPane({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const settings = useSettings(client)
  const update = useUpdateGeneral(client)
  const lifecycle = useLifecycle()
  const [quitError, setQuitError] = useState<unknown>(null)
  const [quitPending, setQuitPending] = useState(false)
  const items = { system: t('system'), 'zh-CN': '简体中文', en: 'English' }
  const snapshot = lifecycle?.snapshot
  const closeItems: Record<CloseBehavior, string> = {
    ask: t('ask'),
    background: t(snapshot?.platform === 'macos' ? 'closeBackgroundMac' : 'closeBackgroundTray'),
    quit: t('closeQuit'),
  }
  const quit = async () => {
    if (!lifecycle || quitPending) return
    setQuitPending(true)
    setQuitError(null)
    try { await lifecycle.client.quit() } catch (cause) { setQuitError(cause) } finally { setQuitPending(false) }
  }
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
    {snapshot && <SettingsGroup title={t('windowBehavior')}>
      <SettingsRow label={t('closeBehavior')} description={t('closeBehaviorHint')} htmlFor="general.closeBehavior">
        <Select value={settings.data?.settings.general.closeBehavior ?? 'ask'} items={closeItems}
          disabled={!settings.data || update.isPending || snapshot.quitting}
          onValueChange={(closeBehavior: CloseBehavior | null) => { if (closeBehavior) update.mutate({ closeBehavior }) }}>
          <SelectTrigger id="general.closeBehavior" className="w-52"><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(closeItems).map(([value, label]) =>
            <SelectItem key={value} value={value} disabled={value === 'background' && !snapshot.backgroundAvailable}>{label}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
      {!snapshot.backgroundAvailable && <p className="text-xs text-muted-foreground">{t('trayUnavailable')}</p>}
      {snapshot.platform === 'macos' && <SettingsRow label={t('showMenuBarIcon')} description={t('showMenuBarIconHint')} htmlFor="general.showMenuBarIcon">
        <Switch id="general.showMenuBarIcon" checked={settings.data?.settings.general.showMenuBarIcon ?? false}
          disabled={!settings.data || update.isPending || snapshot.quitting}
          onCheckedChange={(showMenuBarIcon) => update.mutate({ showMenuBarIcon })} />
      </SettingsRow>}
      <SettingsRow label={t('quitMailx')} description={t('quitMailxHint')}>
        <Button variant="outline" size="sm" disabled={quitPending || snapshot.quitting} onClick={() => void quit()}>{t('quitMailx')}</Button>
      </SettingsRow>
      {quitError !== null && <p role="alert" className="text-sm text-destructive">{errorText(quitError)}</p>}
    </SettingsGroup>}
  </SettingsPage>
}
