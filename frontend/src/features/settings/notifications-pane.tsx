import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { HugeiconsIcon } from '@hugeicons/react'
import { PlayIcon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { sortAccountsForDisplay } from '@/features/accounts/account-order'
import type { Account, NotificationSound } from '@/generated/accounts'
import { useLifecycle } from '@/features/lifecycle/lifecycle-context'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { loadMailTree } from '@/features/mail/mail-tree'
import { SettingsGroup, SettingsLoadError, SettingsPage, SettingsRow, SettingsSaveError } from './settings-layout'
import { useSettings, useUpdateBadge, useUpdateNotifications } from './settings-queries'

/** MailX plays these itself; the others belong to the system or are silent. */
const OWN_SOUNDS: NotificationSound[] = ['chime', 'ding', 'droplet', 'bubble', 'marimba', 'sparkle', 'knock', 'chord']

export function NotificationsPane({ client }: { client: MailxClient }) {
  useTranslation()
  const SOUNDS: Record<NotificationSound, string> = {
    system: t('settings:soundSystem'),
    chime: t('settings:soundChime'),
    ding: t('settings:soundDing'),
    droplet: t('settings:soundDroplet'),
    bubble: t('settings:soundBubble'),
    marimba: t('settings:soundMarimba'),
    sparkle: t('settings:soundSparkle'),
    knock: t('settings:soundKnock'),
    chord: t('settings:soundChord'),
    none: t('settings:soundNone'),
  }
  const settings = useSettings(client)
  const update = useUpdateNotifications(client)
  const lifecycle = useLifecycle()
  // The desktop host says whether it can show a badge; a browser preview without one shows the group.
  const badgeAvailable = lifecycle ? lifecycle.snapshot?.badgeAvailable === true : true
  // The mail tree already holds the accounts and refreshes when they change.
  const accounts = useQuery({ queryKey: mailKeys.tree(), queryFn: () => loadMailTree(client), select: (tree) => tree.accounts })
  if (settings.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (settings.isError) return <SettingsLoadError error={settings.error} onRetry={() => void settings.refetch()} disabled={settings.isFetching} />

  const notifications = settings.data.settings.notifications
  const orderedAccounts = sortAccountsForDisplay(accounts.data ?? [], settings.data.settings.layout.accountOrder)
  const off = !notifications.enabled
  const muted = new Set(notifications.mutedAccounts)
  const setMuted = (accountId: string, mute: boolean) => {
    const next = new Set(muted)
    if (mute) next.add(accountId)
    else next.delete(accountId)
    update.mutate({ mutedAccounts: [...next].sort() })
  }

  return <SettingsPage>
    <SettingsGroup title={t('settings:newMail')}>
      <SettingsRow label={t('settings:notifyMe')} htmlFor="notifications.enabled">
        <Switch id="notifications.enabled" checked={notifications.enabled}
          onCheckedChange={(enabled) => update.mutate({ enabled })} />
      </SettingsRow>
      <SettingsRow label={t('settings:showPreview')} description={t('settings:notificationPreviewHint')} htmlFor="notifications.showPreview">
        <Switch id="notifications.showPreview" checked={notifications.showPreview} disabled={off}
          onCheckedChange={(showPreview) => update.mutate({ showPreview })} />
      </SettingsRow>
      <SettingsRow label={t('settings:whenFocused')} htmlFor="notifications.whenFocused">
        <Switch id="notifications.whenFocused" checked={notifications.whenFocused} disabled={off}
          onCheckedChange={(whenFocused) => update.mutate({ whenFocused })} />
      </SettingsRow>
      <SettingsRow label={t('settings:sound')} htmlFor="notifications.sound">
        {OWN_SOUNDS.includes(notifications.sound) && (
          <Button variant="ghost" size="icon-sm" aria-label={t('settings:previewSound')} disabled={off}
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
      <SettingsRow label={t('settings:testNotification')} description={t('settings:testNotificationHint')}>
        <TestNotification client={client} disabled={off} />
      </SettingsRow>
      <SettingsSaveError update={update} fields={['enabled', 'showPreview', 'whenFocused', 'sound']} />
    </SettingsGroup>
    {orderedAccounts.length > 0 && (
      <SettingsGroup title={t('settings:byAccount')}>
        {orderedAccounts.map((account) => (
          <SettingsRow key={account.id} htmlFor={`notifications.account.${account.id}`}
            label={<AccountLabel account={account} />}
            description={account.emailAddress}>
            <Switch id={`notifications.account.${account.id}`} checked={!muted.has(account.id)} disabled={off}
              onCheckedChange={(on) => setMuted(account.id, !on)} />
          </SettingsRow>
        ))}
        <SettingsSaveError update={update} fields={['mutedAccounts']} />
      </SettingsGroup>
    )}
    {badgeAvailable && <BadgeSettingsGroups client={client} accounts={orderedAccounts} />}
  </SettingsPage>
}

function AccountLabel({ account }: { account: Account }) {
  return <span className="flex min-w-0 items-center gap-2">
    <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-[18px] text-[10px]" />
    <span className="truncate">{account.displayName}</span>
  </span>
}

/** The unread number on the app icon, and which accounts it counts. */
function BadgeSettingsGroups({ client, accounts }: { client: MailxClient; accounts: Account[] }) {
  useTranslation()
  const settings = useSettings(client)
  const update = useUpdateBadge(client)
  if (!settings.data) return null
  const badge = settings.data.settings.badge
  const excluded = new Set(badge.excludedAccounts)
  const setCounted = (accountId: string, counted: boolean) => {
    const next = new Set(excluded)
    if (counted) next.delete(accountId)
    else next.add(accountId)
    update.mutate({ excludedAccounts: [...next].sort() })
  }
  return <>
    <SettingsGroup title={t('settings:appBadge')}>
      <SettingsRow label={t('settings:appBadgeShow')} description={t('settings:appBadgeHint')} htmlFor="badge.enabled">
        <Switch id="badge.enabled" checked={badge.enabled} onCheckedChange={(enabled) => update.mutate({ enabled })} />
      </SettingsRow>
      <SettingsSaveError update={update} fields={['enabled']} />
    </SettingsGroup>
    {accounts.length > 0 && (
      <SettingsGroup title={t('settings:appBadgeAccounts')}>
        {accounts.map((account) => (
          <SettingsRow key={account.id} htmlFor={`badge.account.${account.id}`} label={<AccountLabel account={account} />} description={account.emailAddress}>
            <Switch id={`badge.account.${account.id}`} checked={!excluded.has(account.id)} disabled={!badge.enabled}
              onCheckedChange={(on) => setCounted(account.id, on)} />
          </SettingsRow>
        ))}
        <SettingsSaveError update={update} fields={['excludedAccounts']} />
      </SettingsGroup>
    )}
  </>
}

function TestNotification({ client, disabled }: { client: MailxClient; disabled: boolean }) {
  useTranslation()
  const [state, setState] = useState<'idle' | 'busy' | 'sent' | 'failed'>('idle')
  const send = () => {
    setState('busy')
    client.testNotification().then(() => setState('sent'), () => setState('failed'))
  }
  return (
    <div className="flex items-center gap-3">
      {state === 'sent' && <span role="status" className="text-xs text-muted-foreground">{t('settings:sent')}</span>}
      {state === 'failed' && <span role="alert" className="text-xs text-destructive">{t('settings:sendFailed')}</span>}
      <Button variant="outline" size="sm" disabled={disabled || state === 'busy'} onClick={send}>{t('settings:sendTest')}</Button>
    </div>
  )
}
