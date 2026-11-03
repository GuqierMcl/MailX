import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Switch } from '@/components/ui/switch'
import { toast } from '@/components/ui/toast'
import { mcpClientGroups, mcpStatusText } from '@/features/mcp/mcp-status-text'
import { useMcpStatus } from '@/features/mcp/use-mcp-status'
import type { MailxClient } from '@/lib/mailx-client'
import { settingsKeys } from '@/lib/query-client'
import { mcpClientSnippet } from '@/features/mcp/client-snippet'
import { errorText } from '@/i18n/errors'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings } from './settings-queries'

export function McpPane({ client }: { client: MailxClient }) {
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
  const copyPath = async (label: string, path: string) => {
    try {
      await navigator.clipboard.writeText(path)
      toast.success(t('mcpPathCopied', { label }))
    } catch (cause) {
      toast.error(t('mcpCopyFailed'), { description: errorText(cause) })
    }
  }
  const clients = status?.state === 'running' ? mcpClientGroups(status.clients) : []

  return <SettingsPage>
    {(settings.isError || error !== null) && <Alert variant="destructive">
      <AlertTitle>{t(settings.isError ? 'readFailed' : 'mcpToggleFailed')}</AlertTitle>
      <AlertDescription>{errorText(settings.error ?? error)}</AlertDescription>
    </Alert>}
    <SettingsGroup title={t('mcp')}>
      <SettingsRow label={t('mcpEnable')} htmlFor="mcp.enabled">
        <Switch id="mcp.enabled" checked={enabled} disabled={!settings.data || pending}
          onCheckedChange={(next) => { if (next) setConfirming(true); else void apply(false) }} />
      </SettingsRow>
      <SettingsRow label={t('mcpStatus')}>
        <span role="status" className="text-sm text-muted-foreground">{mcpStatusText(status, enabled)}</span>
      </SettingsRow>
      {clients.length > 0 && <ul aria-label={t('mcpClientsConnected')} className="space-y-0.5 text-xs text-muted-foreground">
        {clients.map((name) => <li key={name}>{name}</li>)}
      </ul>}
    </SettingsGroup>
    <SettingsGroup title={t('mcpClients')} description={t('mcpClientsHint')}>
      <div>
        <Button variant="outline" size="sm" disabled={config.isError} onClick={() => void copy()}>{t('mcpCopyConfig')}</Button>
      </div>
      {config.isError
        ? <p role="alert" className="text-sm text-destructive">{errorText(config.error)}</p>
        : <Collapsible>
          <CollapsibleTrigger className="text-xs text-muted-foreground hover:text-foreground">{t('mcpManualSetup')}</CollapsibleTrigger>
          <CollapsibleContent className="mt-2 flex flex-col gap-2">
            {config.data && ([
              { id: 'command', label: t('mcpCommandPath'), shown: config.data.display.command, full: config.data.command },
              { id: 'discovery', label: t('mcpDiscoveryPath'), shown: config.data.display.discovery, full: config.data.args[1] ?? '' },
            ] as const).map((item) => <div key={item.id} className="flex items-center justify-between gap-2">
              <div className="flex min-w-0 flex-col gap-1">
                <span className="text-xs text-muted-foreground">{item.label}</span>
                <code className="break-all font-mono text-xs select-text" data-testid={`mcp-${item.id}-path`}>{item.shown}</code>
              </div>
              <Button variant="outline" size="sm" aria-label={t('mcpCopyPath', { label: item.label })} onClick={() => void copyPath(item.label, item.full)}>{t('mcpCopyShort')}</Button>
            </div>)}
          </CollapsibleContent>
        </Collapsible>}
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
