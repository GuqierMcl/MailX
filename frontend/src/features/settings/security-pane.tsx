import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import type { RemoteImagePolicy } from '@/generated/accounts'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateSecurity } from './settings-queries'

const POLICIES: Record<RemoteImagePolicy, string> = { ask: '每次询问', automatic: '自动加载', never: '永不加载' }

export function SecurityPane({ client }: { client: MailxClient }) {
  const settings = useSettings(client)
  const update = useUpdateSecurity(client)
  if (settings.isPending) return <div className="flex justify-center p-10"><Spinner className="size-5" /></div>
  if (settings.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>无法读取设置</AlertTitle><AlertDescription>{clientFailure(settings.error).message}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void settings.refetch()}>重试</Button>
  </SettingsPage>
  return <SettingsPage>
    {update.isError && <Alert variant="destructive"><AlertTitle>设置保存失败</AlertTitle><AlertDescription>{clientFailure(update.error).message}</AlertDescription></Alert>}
    <SettingsGroup title="邮件图片">
      <SettingsRow label="加载远程图片" htmlFor="security.remoteImages" description="远程图片可能让发件人获知你已阅读邮件。邮件自带的内嵌图片始终显示。">
        <Select value={settings.data.settings.security.remoteImages} items={POLICIES} disabled={update.isPending}
          onValueChange={(remoteImages: RemoteImagePolicy | null) => { if (remoteImages) update.mutate({ remoteImages }) }}>
          <SelectTrigger id="security.remoteImages" className="w-36"><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(POLICIES).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
}
