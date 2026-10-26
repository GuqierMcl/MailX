import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import type { MailxClient } from '@/lib/mailx-client'
import { useSettings, useUpdateGeneral } from '@/features/settings/settings-queries'
import { SettingsRow } from '@/features/settings/settings-layout'
import { useUpdater } from './updater-context'

/** What the About page says about the last check, with a way to check again. */
export function UpdateStatusRow() {
  useTranslation()
  const { snapshot, checked, check, openDialog } = useUpdater()
  const version = snapshot.phase === 'available' || snapshot.phase === 'ready' ? snapshot.version : null
  const status = (() => {
    switch (snapshot.phase) {
      case 'checking':
        return <span className="flex items-center gap-2 text-muted-foreground"><Spinner className="size-3" />{t('updater:statusChecking')}</span>
      case 'available':
        return <span className="text-primary">{t('updater:statusAvailable', { version })}</span>
      case 'downloading':
        return <span className="text-muted-foreground">{t('updater:statusDownloading', { version: snapshot.version })}</span>
      case 'ready':
        return <span className="text-primary">{t('updater:statusReady', { version: snapshot.version })}</span>
      case 'failed':
        return <span className="text-destructive">{t(snapshot.stage === 'check' ? 'updater:checkFailed' : 'updater:downloadFailed')}</span>
      default:
        return checked ? <span className="text-muted-foreground">{t('updater:statusLatest')}</span> : null
    }
  })()
  const busy = snapshot.phase === 'checking' || snapshot.phase === 'downloading'
  return (
    <SettingsRow label={t('updater:statusLabel')}>
      {status && (version
        ? <button type="button" className="text-xs underline-offset-2 hover:underline" onClick={openDialog}>{status}</button>
        : <span className="text-xs">{status}</span>)}
      {snapshot.phase === 'failed' && snapshot.stage === 'check' && (
        <Button variant="outline" size="sm" onClick={() => void check()}>{t('updater:retry')}</Button>
      )}
      <Button variant="outline" size="sm" disabled={busy} onClick={() => void check()}>{t('updater:checkNow')}</Button>
    </SettingsRow>
  )
}

/** The switch that stops the start-up and periodic checks; the manual one stays. */
export function AutoCheckRow({ client }: { client: MailxClient }) {
  useTranslation()
  const settings = useSettings(client)
  const update = useUpdateGeneral(client)
  const general = settings.data?.settings.general
  return (
    <SettingsRow label={t('updater:autoCheck')} description={t('updater:autoCheckDescription')} htmlFor="general.autoCheckUpdates">
      <Switch id="general.autoCheckUpdates" checked={general?.autoCheckUpdates ?? true}
        disabled={!general || update.isPending}
        onCheckedChange={(checked: boolean) => update.mutate({ autoCheckUpdates: checked })} />
    </SettingsRow>
  )
}
