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
    <SettingsGroup title="邮件图片">
      <SettingsRow label="加载远程图片" htmlFor="security.remoteImages" description="远程图片可能让发件人获知你已阅读邮件。邮件自带的内嵌图片始终显示。">
        <Select value={settings.data.settings.security.remoteImages} items={POLICIES} disabled={update.isPending}
          onValueChange={(remoteImages: RemoteImagePolicy | null) => { if (remoteImages) update.mutate({ remoteImages }) }}>
          <SelectTrigger id="security.remoteImages" className="w-36"><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(POLICIES).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="发件人头像">
      <SettingsRow label="获取发件人头像与企业图标" htmlFor="security.senderAvatars"
        description="开启后会联网查找头像：先把发件人邮箱地址的 SHA-256 摘要发给 Gravatar；发件人不是常见个人邮箱（如 Gmail、QQ、163）时，再把发件人域名以明文发给 DuckDuckGo，查不到才发给 Google，以取得该域名的网站图标。这些服务因此可能得知你收到了哪些来源的邮件。请求都走该账号的网络路由；使用自定义代理的账号不会获取。关闭时只显示首字母头像，不发送任何请求。">
        <Switch id="security.senderAvatars" checked={settings.data.settings.security.senderAvatars} disabled={update.isPending}
          onCheckedChange={(senderAvatars) => update.mutate({ senderAvatars })} />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="图片缓存">
      <SettingsRow label="已缓存的图片" description="头像、网站图标和已下载的邮件图片会保存在本机的数据目录（最多约 200 MB，较久未用的先清理），再次打开时无需重新下载。清除后会在需要时重新获取。">
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
