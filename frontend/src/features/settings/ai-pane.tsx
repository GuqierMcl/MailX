import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, Delete02Icon, Edit02Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Spinner } from '@/components/ui/spinner'
import type { AiConnectionState, AiState } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { aiKeys } from '@/lib/query-client'
import { AiProviderIcon } from './ai-provider-icon'
import { AiProviderDialog } from './ai-provider-dialog'
import { useAiCatalog } from './ai-queries'
import { ModelList, type ModelGroup } from './ai-model-picker'
import { ProxyPasswordFields, ProxyRows } from './proxy-rows'
import { SettingsDisclosure, SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'

type Dialog = { kind: 'add' } | { kind: 'edit'; connection: AiConnectionState }

const hostOf = (baseUrl: string) => {
  try { return new URL(baseUrl).host } catch { return baseUrl }
}

export function AiPane({ client }: { client: MailxClient }) {
  useTranslation()
  const queryClient = useQueryClient()
  const state = useQuery({ queryKey: aiKeys.state(), queryFn: () => client.aiState() })
  const catalog = useAiCatalog(client)
  const [dialog, setDialog] = useState<Dialog | null>(null)
  const [removing, setRemoving] = useState<AiConnectionState | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const publish = (next: AiState) => queryClient.setQueryData(aiKeys.state(), next)
  const attempt = (run: Promise<AiState>) => {
    setFailure(null)
    return run.then(publish, (error: unknown) => setFailure(errorText(error)))
  }
  if (state.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (state.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>{t('settings:aiReadFailed')}</AlertTitle><AlertDescription>{errorText(state.error)}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void state.refetch()}>{t('settings:retry')}</Button>
  </SettingsPage>
  const ai = state.data
  const localIds = new Set(catalog.data?.providers.filter((provider) => provider.local).map((provider) => provider.id))
  const addButton = <Button variant="outline" size="sm" onClick={() => setDialog({ kind: 'add' })}>{t('settings:aiAdd')}</Button>
  return <SettingsPage>
    {failure && <Alert variant="destructive"><AlertTitle>{t('settings:saveFailedTitle')}</AlertTitle><AlertDescription>{failure}</AlertDescription></Alert>}
    <SettingsGroup title={t('settings:aiProviders')}>
      {ai.connections.length === 0 ? <div className="flex flex-col items-start gap-3">
        <p className="text-sm text-muted-foreground">{t('settings:aiEmpty')}</p>
        {addButton}
      </div> : <>
        <ul className="flex flex-col gap-1">
          {ai.connections.map((connection) => (
            <li key={connection.id} className="flex items-center gap-3 rounded-lg py-1.5">
              <AiProviderIcon client={client} providerId={connection.providerId} name={connection.name} />
              <div className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-sm font-medium">{connection.name}</span>
                <span className="truncate text-xs text-muted-foreground">{hostOf(connection.baseUrl)}</span>
              </div>
              {!connection.hasKey && !localIds.has(connection.providerId) && <span className="shrink-0 text-xs text-muted-foreground">{t('settings:aiNoKey')}</span>}
              <Button variant="ghost" size="icon-sm" aria-label={t('settings:aiEdit', { name: connection.name })} onClick={() => setDialog({ kind: 'edit', connection })}>
                <HugeiconsIcon icon={Edit02Icon} strokeWidth={2} />
              </Button>
              <Button variant="ghost" size="icon-sm" aria-label={t('settings:aiRemove', { name: connection.name })} onClick={() => setRemoving(connection)}>
                <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
              </Button>
            </li>
          ))}
        </ul>
        <div>{addButton}</div>
      </>}
    </SettingsGroup>
    <SettingsGroup title={t('settings:aiDefaultModel')}>
      <DefaultModelRow client={client} ai={ai} onPick={(model) => attempt(client.aiSetDefaultModel(model))} />
    </SettingsGroup>
    <SettingsDisclosure title={t('settings:aiAdvanced')}
      summary={ai.route.kind === 'proxy' ? t('settings:translationAdvancedProxy', { host: ai.route.host, port: ai.route.port }) : t('settings:translationNetworkDirect')}>
      <ProxyRows idPrefix="ai" route={ai.route} networkDescription={t('settings:aiNetworkDescription')}
        onRouteChange={(route) => void attempt(client.aiSetRoute(route))}
        password={(hasUsername) => <ProxyPasswordFields id="ai.proxyPassword" hasUsername={hasUsername} configured={ai.proxyPasswordSet}
          onSave={(value) => client.aiSetProxyPassword(value).then(publish)} onRemove={() => client.aiRemoveProxyPassword().then(publish)} />} />
    </SettingsDisclosure>
    {dialog && <AiProviderDialog client={client} state={ai} editing={dialog.kind === 'edit' ? dialog.connection : null} onState={publish} onClose={() => setDialog(null)} />}
    <AlertDialog open={removing !== null} onOpenChange={(open) => !open && setRemoving(null)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('settings:aiRemoveTitle', { name: removing?.name })}</AlertDialogTitle>
          <AlertDialogDescription>{t('settings:aiRemoveBody')}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('settings:aiCancel')}</AlertDialogCancel>
          <AlertDialogAction onClick={() => { if (removing) void attempt(client.aiRemoveConnection(removing.id)); setRemoving(null) }}>{t('settings:aiRemoveConfirm')}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </SettingsPage>
}

type TestState = { kind: 'idle' } | { kind: 'running' } | { kind: 'works'; seconds: string } | { kind: 'failed'; reason: string }

function DefaultModelRow({ client, ai, onPick }: { client: MailxClient; ai: AiState; onPick: (model: NonNullable<AiState['defaultModel']>) => Promise<unknown> }) {
  useTranslation()
  const [open, setOpen] = useState(false)
  const [test, setTest] = useState<TestState>({ kind: 'idle' })
  const current = ai.defaultModel
  const currentConnection = ai.connections.find((connection) => connection.id === current?.connectionId)
  // Lists are fetched live each time the picker opens and are not kept.
  const lists = useQueries({
    queries: ai.connections.map((connection) => ({
      queryKey: aiKeys.models(connection.id), queryFn: () => client.aiListModels(connection.id),
      enabled: open, staleTime: 0, gcTime: 60_000, refetchOnMount: 'always' as const,
    })),
  })
  const groups: ModelGroup[] = ai.connections.map((connection, index) => {
    const list = lists[index]
    return { id: connection.id, name: connection.name, providerId: connection.providerId, models: list.data, loading: list.isFetching && !list.data, failure: list.isError ? errorText(list.error) : null }
  })
  const run = () => {
    if (!current) return
    setTest({ kind: 'running' })
    client.aiTest(current.connectionId, current.modelId).then(
      ({ latencyMs }) => setTest({ kind: 'works', seconds: (latencyMs / 1000).toFixed(1) }),
      (error: unknown) => setTest({ kind: 'failed', reason: errorText(error) }),
    )
  }
  const none = ai.connections.length === 0
  return <SettingsRow label={t('settings:aiModel')} htmlFor="ai.model" description={none ? t('settings:aiModelNeedsProvider') : t('settings:aiDefaultModelDescription')}>
    <div className="flex flex-col items-end gap-1.5">
      <div className="flex items-center gap-2">
        <Popover open={open} onOpenChange={setOpen}>
          <PopoverTrigger render={<Button id="ai.model" role="combobox" aria-label={t('settings:aiDefaultModel')} aria-expanded={open} variant="outline" className="w-56 justify-between font-normal" disabled={none} />}>
            <span className="truncate">{current ? current.name : t('settings:aiModelPlaceholder')}</span>
            <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} className="size-4 shrink-0 text-muted-foreground" />
          </PopoverTrigger>
          <PopoverContent align="end" className="w-80 gap-0 p-0">
            <ModelList client={client} groups={groups} onPick={(model) => { setOpen(false); setTest({ kind: 'idle' }); void onPick(model) }} />
          </PopoverContent>
        </Popover>
        <Button variant="outline" size="sm" disabled={!current || !currentConnection || test.kind === 'running'} onClick={run}>
          {test.kind === 'running' ? t('settings:aiTestRunning') : t('settings:aiTest')}
        </Button>
      </div>
      {test.kind === 'works' && <span role="status" className="text-xs text-muted-foreground">{t('settings:aiTestWorks', { seconds: test.seconds })}</span>}
      {test.kind === 'failed' && <span role="alert" className="max-w-72 text-right text-xs text-destructive">{test.reason}</span>}
    </div>
  </SettingsRow>
}
