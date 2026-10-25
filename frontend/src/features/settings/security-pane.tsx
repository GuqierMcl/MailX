import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { useState } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Delayed } from '@/components/ui/delayed'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import type { RemoteImagePolicy } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateSecurity } from './settings-queries'



export function SecurityPane({ client }: { client: MailxClient }) {
  useTranslation()
  const POLICIES: Record<RemoteImagePolicy, string> = { ask: t('settings:ask'), automatic: t('settings:automatic'), never: t('settings:never') }
  const settings = useSettings(client)
  const update = useUpdateSecurity(client)
  if (settings.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (settings.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>{t('settings:readFailed')}</AlertTitle><AlertDescription>{errorText(settings.error)}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void settings.refetch()}>{t('settings:retry')}</Button>
  </SettingsPage>
  return <SettingsPage>
    {update.isError && <Alert variant="destructive"><AlertTitle>{t('settings:saveFailedTitle')}</AlertTitle><AlertDescription>{errorText(update.error)}</AlertDescription></Alert>}
    <SettingsGroup title={t('settings:images')}>
      <SettingsRow label={t('settings:remoteImages')} htmlFor="security.remoteImages" description={t('settings:remoteImagesDescription')}>
        <Select value={settings.data.settings.security.remoteImages} items={POLICIES} disabled={update.isPending}
          onValueChange={(remoteImages: RemoteImagePolicy | null) => { if (remoteImages) update.mutate({ remoteImages }) }}>
          <SelectTrigger id="security.remoteImages" className="w-36"><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(POLICIES).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
      <SettingsRow label={t('settings:avatars')} htmlFor="security.senderAvatars"
        description={t('settings:avatarsDescription')}>
        <Switch id="security.senderAvatars" checked={settings.data.settings.security.senderAvatars} disabled={update.isPending}
          onCheckedChange={(senderAvatars) => update.mutate({ senderAvatars })} />
      </SettingsRow>
      <SettingsRow label={t('settings:savedImages')}>
        <ClearImageCache client={client} />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
}

function ClearImageCache({ client }: { client: MailxClient }) {
  useTranslation()
  const [state, setState] = useState<'idle' | 'busy' | 'done' | 'failed'>('idle')
  const clear = () => {
    setState('busy')
    client.clearImageCache().then(() => setState('done'), () => setState('failed'))
  }
  return (
    <div className="flex items-center gap-3">
      {state === 'done' && <span role="status" className="text-xs text-muted-foreground">{t('settings:cacheCleared')}</span>}
      {state === 'failed' && <span role="alert" className="text-xs text-destructive">{t('settings:cacheClearFailed')}</span>}
      <Button variant="outline" size="sm" disabled={state === 'busy'} onClick={clear}>{t('settings:clearCache')}</Button>
    </div>
  )
}
