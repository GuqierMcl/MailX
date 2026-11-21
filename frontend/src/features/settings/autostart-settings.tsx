import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { ErrorNotice } from '@/components/error-notice'
import { Switch } from '@/components/ui/switch'
import { errorText } from '@/i18n/errors'
import type { MailxClient } from '@/lib/mailx-client'
import { SettingsGroup, SettingsRow, SettingsSaveError } from './settings-layout'
import { useSettings, useUpdateGeneral } from './settings-queries'

const key = ['desktop', 'autostart'] as const

export function AutostartSettings({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const cache = useQueryClient()
  const settings = useSettings(client)
  const general = useUpdateGeneral(client)
  const status = useQuery({
    queryKey: key, queryFn: () => client.getAutostart(),
    staleTime: 0, refetchOnMount: 'always', refetchOnWindowFocus: 'always', retry: false,
  })
  const change = useMutation({
    mutationFn: (enabled: boolean) => client.setAutostart(enabled),
    onMutate: async () => { await cache.cancelQueries({ queryKey: key }) },
    onSuccess: async (snapshot) => {
      await cache.cancelQueries({ queryKey: key })
      cache.setQueryData(key, snapshot)
    },
    // A failed system operation can have partially changed the registration.
    onError: async () => { await cache.invalidateQueries({ queryKey: key }) },
  })
  const { refetch } = status
  useEffect(() => {
    const refresh = () => { void refetch() }
    window.addEventListener('focus', refresh)
    return () => window.removeEventListener('focus', refresh)
  }, [refetch])
  const known = !!status.data && !status.isError
  const enabled = known && status.data.enabled
  const pending = change.isPending || status.isFetching
  return <SettingsGroup title={t('startup')}>
    <SettingsRow label={t('autostart')} description={t('autostartHint')} htmlFor="desktop.autostart">
      <Switch id="desktop.autostart" checked={enabled}
        disabled={!known || !status.data?.available || pending}
        onCheckedChange={(next) => change.mutate(next)} />
    </SettingsRow>
    {(status.isError || change.isError) && <ErrorNotice message={t(status.isError ? 'autostartReadFailed' : 'autostartSaveFailed')}
      detail={errorText(status.error ?? change.error)} actionLabel={status.isError ? t('autostartRetry') : undefined}
      onAction={status.isError ? () => void refetch() : undefined} actionDisabled={pending} />}
    {status.isError && <p role="status" className="text-xs text-muted-foreground">{t('autostartUnknown')}</p>}
    {known && !status.data.available && <p className="text-xs text-muted-foreground">{t('autostartUnavailable')}</p>}
    <SettingsRow label={t('silentStart')} description={t('silentStartHint')} htmlFor="general.silentStart">
      <Switch id="general.silentStart" checked={settings.data?.settings.general.silentStart ?? false}
        disabled={!enabled || !status.data?.available || pending || !settings.data || general.isPending}
        onCheckedChange={(silentStart) => general.mutate({ silentStart })} />
    </SettingsRow>
    <SettingsSaveError update={general} message={t('autostartSaveFailed')} />
  </SettingsGroup>
}
