import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { toast } from '@/components/ui/toast'
import { mcpStatusText } from '@/features/mcp/mcp-status-text'
import { useMcpStatus } from '@/features/mcp/use-mcp-status'
import type { MailxClient } from '@/lib/mailx-client'
import { settingsKeys } from '@/lib/query-client'
import { mcpClientSnippet } from '@/features/mcp/client-snippet'
import { errorText } from '@/i18n/errors'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings } from './settings-queries'
import type { SettingsSection } from './settings-dialog'

export function McpPane({ client, onOpenSection }: { client: MailxClient; onOpenSection: (section: SettingsSection) => void }) {
  const { t } = useTranslation('settings')
  const queryClient = useQueryClient()
  const settings = useSettings(client)
  const status = useMcpStatus(client)
  const enabled = settings.data?.settings.mcp.enabled ?? false
  const [confirming, setConfirming] = useState(false)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const config = useQuery({ queryKey: ['mcp', 'clientConfig'], queryFn: () => client.getMcpClientConfig(), retry: false, staleTime: Infinity })

  const apply = async (next: boolean) => {
    setConfirming(false)
    setPending(true)
    setError(null)
    try {
      await client.setMcpEnabled(next)
      await queryClient.invalidateQueries({ queryKey: settingsKeys.all })
    } catch (cause) {
      setError(cause)
    } finally {
      setPending(false)
    }
  }
  const copy = async () => {
    try {
      const snippet = mcpClientSnippet(config.data ?? await client.getMcpClientConfig())
      await navigator.clipboard.writeText(snippet)
      toast.success(t('mcpCopied'))
    } catch (cause) {
      toast.error(t('mcpCopyFailed'), { description: errorText(cause) })
    }
  }
  const running = status?.state === 'running' ? status : null

  return <SettingsPage>
    {(settings.isError || error !== null) && <Alert variant="destructive">
      <AlertTitle>{t(settings.isError ? 'readFailed' : 'mcpToggleFailed')}</AlertTitle>
      <AlertDescription>{errorText(settings.error ?? error)}</AlertDescription>
    </Alert>}
    <SettingsGroup title={t('mcp')} description={t('mcpIntro')}>
      <SettingsRow label={t('mcpEnable')} description={t('mcpEnableHint')} htmlFor="mcp.enabled">
        <Switch id="mcp.enabled" checked={enabled} disabled={!settings.data || pending}
          onCheckedChange={(next) => { if (next) setConfirming(true); else void apply(false) }} />
      </SettingsRow>
      <SettingsRow label={t('mcpStatus')}>
        <span role="status" className="text-sm text-muted-foreground">{mcpStatusText(status, enabled)}</span>
      </SettingsRow>
      {running?.lastClientName && <p className="text-xs text-muted-foreground">
        {t('mcpLastClient', { name: running.lastClientName })}
      </p>}
    </SettingsGroup>
    <SettingsGroup title={t('mcpCapabilities')}>
      <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">
        <li>{t('mcpCan')}</li>
        <li>{t('mcpCannot')}</li>
        <li>{t('mcpLocalAccess')}</li>
      </ul>
      <p className="text-sm text-muted-foreground">{t('mcpLifecycle')}</p>
      <div><Button variant="link" size="sm" className="h-auto px-0" onClick={() => onOpenSection('general')}>{t('mcpOpenGeneral')}</Button></div>
    </SettingsGroup>
    <SettingsGroup title={t('mcpClients')} description={t('mcpClientsHint')}>
      <SettingsRow label={t('mcpCopyConfig')} description={t('mcpCopyConfigHint')}>
        <Button variant="outline" size="sm" disabled={config.isError} onClick={() => void copy()}>{t('mcpCopyConfig')}</Button>
      </SettingsRow>
      <div className="flex flex-col gap-1">
        <span className="text-xs text-muted-foreground">{t('mcpCommandPath')}</span>
        {config.isError
          ? <p role="alert" className="text-sm text-destructive">{errorText(config.error)}</p>
          : <code className="break-all font-mono text-xs select-text" data-testid="mcp-command-path">{config.data?.command ?? '…'}</code>}
      </div>
    </SettingsGroup>
    <AlertDialog open={confirming} onOpenChange={setConfirming}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('mcpConfirmTitle')}</AlertDialogTitle>
          <AlertDialogDescription render={<div />} className="space-y-2">
            <p>{t('mcpConfirmAccess')}</p>
            <p>{t('mcpConfirmLimits')}</p>
            <p>{t('mcpConfirmLocal')}</p>
            <p>{t('mcpConfirmOff')}</p>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('mcpConfirmCancel')}</AlertDialogCancel>
          <AlertDialogAction onClick={() => void apply(true)}>{t('mcpConfirmEnable')}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </SettingsPage>
}
