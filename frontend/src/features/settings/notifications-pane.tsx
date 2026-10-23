import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { HugeiconsIcon } from '@hugeicons/react'
import { PlayIcon } from '@hugeicons/core-free-icons'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import type { NotificationSound } from '@/generated/accounts'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { loadMailTree } from '@/features/mail/mail-tree'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateNotifications } from './settings-queries'

const SOUNDS: Record<NotificationSound, string> = {
  system: '系统默认',
  chime: '清铃',
  ding: '叮',
  droplet: '水滴',
  bubble: '气泡',
  marimba: '木琴',
  sparkle: '星光',
  knock: '轻叩',
  chord: '和弦',
  none: '无声',
}

/** MailX plays these itself; the others belong to the system or are silent. */
const OWN_SOUNDS: NotificationSound[] = ['chime', 'ding', 'droplet', 'bubble', 'marimba', 'sparkle', 'knock', 'chord']

export function NotificationsPane({ client }: { client: MailxClient }) {
  const settings = useSettings(client)
  const update = useUpdateNotifications(client)
  // The mail tree already holds the accounts and refreshes when they change.
  const accounts = useQuery({ queryKey: mailKeys.tree(), queryFn: () => loadMailTree(client), select: (tree) => tree.accounts })
  if (settings.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (settings.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>无法读取设置</AlertTitle><AlertDescription>{clientFailure(settings.error).message}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void settings.refetch()}>重试</Button>
  </SettingsPage>

  const notifications = settings.data.settings.notifications
  const off = !notifications.enabled
  const muted = new Set(notifications.mutedAccounts)
  const setMuted = (accountId: string, mute: boolean) => {
    const next = new Set(muted)
    if (mute) next.add(accountId)
    else next.delete(accountId)
    update.mutate({ mutedAccounts: [...next].sort() })
  }

  return <SettingsPage>
    {update.isError && <Alert variant="destructive"><AlertTitle>设置保存失败</AlertTitle><AlertDescription>{clientFailure(update.error).message}</AlertDescription></Alert>}
    <SettingsGroup title="新邮件通知">
      <SettingsRow label="显示通知" htmlFor="notifications.enabled"
        description="收件箱收到未读新邮件时，通过系统通知提醒。首次同步、补齐的旧邮件、已在其他设备读过的邮件不会提醒。">
        <Switch id="notifications.enabled" checked={notifications.enabled}
          onCheckedChange={(enabled) => update.mutate({ enabled })} />
      </SettingsRow>
      <SettingsRow label="显示发件人和主题" htmlFor="notifications.showPreview"
        description="关闭后通知只显示账号名称和新邮件数量，锁屏或共享屏幕时不会露出邮件内容。">
        <Switch id="notifications.showPreview" checked={notifications.showPreview} disabled={off}
          onCheckedChange={(showPreview) => update.mutate({ showPreview })} />
      </SettingsRow>
      <SettingsRow label="MailX 在前台时也通知" htmlFor="notifications.whenFocused"
        description="关闭时，正在使用 MailX 期间只更新未读数，不弹出通知。">
        <Switch id="notifications.whenFocused" checked={notifications.whenFocused} disabled={off}
          onCheckedChange={(whenFocused) => update.mutate({ whenFocused })} />
      </SettingsRow>
      <SettingsRow label="提示音" htmlFor="notifications.sound"
        description="“系统默认”使用系统的通知提示音；其他音效由 MailX 播放，系统通知本身保持静音。">
        {OWN_SOUNDS.includes(notifications.sound) && (
          <Button variant="ghost" size="icon-sm" aria-label="试听" disabled={off}
            onClick={() => void client.previewSound(notifications.sound).catch(() => undefined)}>
            <HugeiconsIcon icon={PlayIcon} />
          </Button>
        )}
        <Select value={notifications.sound} items={SOUNDS} disabled={off}
          onValueChange={(sound: NotificationSound | null) => {
            if (!sound) return
            update.mutate({ sound })
            // Hearing a sound is the point of choosing it.
            if (OWN_SOUNDS.includes(sound)) void client.previewSound(sound).catch(() => undefined)
          }}>
          <SelectTrigger id="notifications.sound" className="w-32"><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(SOUNDS).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
      <SettingsRow label="测试通知" description="按当前设置发送一条示例通知。若没有出现，请在系统设置中允许 MailX 发送通知。">
        <TestNotification client={client} disabled={off} />
      </SettingsRow>
    </SettingsGroup>
    {accounts.data && accounts.data.length > 0 && (
      <SettingsGroup title="按账号" description="关闭的账号收到新邮件时不会通知，未读数照常更新。">
        {accounts.data.map((account) => (
          <SettingsRow key={account.id} htmlFor={`notifications.account.${account.id}`}
            label={<span className="flex min-w-0 items-center gap-2">
              <AccountAvatar displayName={account.displayName} color={account.color} className="size-[18px] text-[10px]" />
              <span className="truncate">{account.displayName}</span>
            </span>}
            description={account.emailAddress}>
            <Switch id={`notifications.account.${account.id}`} checked={!muted.has(account.id)} disabled={off}
              onCheckedChange={(on) => setMuted(account.id, !on)} />
          </SettingsRow>
        ))}
      </SettingsGroup>
    )}
  </SettingsPage>
}

function TestNotification({ client, disabled }: { client: MailxClient; disabled: boolean }) {
  const [state, setState] = useState<'idle' | 'busy' | 'sent' | 'failed'>('idle')
  const send = () => {
    setState('busy')
    client.testNotification().then(() => setState('sent'), () => setState('failed'))
  }
  return (
    <div className="flex items-center gap-3">
      {state === 'sent' && <span role="status" className="text-xs text-muted-foreground">已发送</span>}
      {state === 'failed' && <span role="alert" className="text-xs text-destructive">发送失败</span>}
      <Button variant="outline" size="sm" disabled={disabled || state === 'busy'} onClick={send}>发送测试通知</Button>
    </div>
  )
}
