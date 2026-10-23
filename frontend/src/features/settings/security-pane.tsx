import { useState } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Delayed } from '@/components/ui/delayed'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import type { RemoteImagePolicy } from '@/generated/accounts'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateSecurity } from './settings-queries'

const POLICIES: Record<RemoteImagePolicy, string> = { ask: '每次询问', automatic: '自动加载', never: '永不加载' }

export function SecurityPane({ client }: { client: MailxClient }) {
  const settings = useSettings(client)
  const update = useUpdateSecurity(client)
  if (settings.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (settings.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>无法读取设置</AlertTitle><AlertDescription>{clientFailure(settings.error).message}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void settings.refetch()}>重试</Button>
  </SettingsPage>
  return <SettingsPage>
    {update.isError && <Alert variant="destructive"><AlertTitle>设置保存失败</AlertTitle><AlertDescription>{clientFailure(update.error).message}</AlertDescription></Alert>}
    <SettingsGroup title="图片">
      <SettingsRow label="邮件里的网络图片" htmlFor="security.remoteImages" description="加载后，发件人可能知道你读过这封邮件。">
        <Select value={settings.data.settings.security.remoteImages} items={POLICIES} disabled={update.isPending}
          onValueChange={(remoteImages: RemoteImagePolicy | null) => { if (remoteImages) update.mutate({ remoteImages }) }}>
          <SelectTrigger id="security.remoteImages" className="w-36"><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(POLICIES).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
      <SettingsRow label="显示发件人头像" htmlFor="security.senderAvatars"
        description="需要联网查找，查找服务会得知你收到了谁的邮件。">
        <Switch id="security.senderAvatars" checked={settings.data.settings.security.senderAvatars} disabled={update.isPending}
          onCheckedChange={(senderAvatars) => update.mutate({ senderAvatars })} />
      </SettingsRow>
      <SettingsRow label="已保存的图片">
        <ClearImageCache client={client} />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
}

function ClearImageCache({ client }: { client: MailxClient }) {
  const [state, setState] = useState<'idle' | 'busy' | 'done' | 'failed'>('idle')
  const clear = () => {
    setState('busy')
    client.clearImageCache().then(() => setState('done'), () => setState('failed'))
  }
  return (
    <div className="flex items-center gap-3">
      {state === 'done' && <span role="status" className="text-xs text-muted-foreground">已清除</span>}
      {state === 'failed' && <span role="alert" className="text-xs text-destructive">清除失败</span>}
      <Button variant="outline" size="sm" disabled={state === 'busy'} onClick={clear}>清除缓存</Button>
    </div>
  )
}
