import { useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery } from '@tanstack/react-query'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { Command, CommandGroup, CommandInput, CommandItem, CommandList } from '@/components/ui/command'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Field, FieldContent, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import type { AiCatalogProvider, AiConnectionState, AiModel, AiProtocol, AiState } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { aiKeys } from '@/lib/query-client'
import { useAiCatalog } from './ai-queries'
import { AiProviderIcon } from './ai-provider-icon'
import { ModelList } from './ai-model-picker'

const CUSTOM_ID = 'custom'

type Step = { kind: 'pick' } | { kind: 'form'; provider: AiCatalogProvider } | { kind: 'model'; connectionId: string; name: string; providerId: string; models: AiModel[] | null }

/** Adds a provider (pick, fill in, choose a model) or edits one (just the form). Mount it to open it; unmount to close. */
export function AiProviderDialog({ client, state, editing, onState, onClose }: {
  client: MailxClient
  state: AiState
  editing: AiConnectionState | null
  onState: (state: AiState) => void
  onClose: () => void
}) {
  useTranslation()
  const catalog = useAiCatalog(client, editing === null)
  const [step, setStep] = useState<Step>(() => editing === null ? { kind: 'pick' } : { kind: 'form', provider: {
    id: editing.providerId, name: editing.name, protocol: editing.protocol, baseUrl: editing.baseUrl, builtIn: false,
  } })
  const title = editing ? t('settings:aiDialogEdit') : step.kind === 'model' ? t('settings:aiDialogModel') : t('settings:aiDialogAdd')
  return <Dialog open onOpenChange={(open) => !open && onClose()}>
    <DialogContent className="flex max-h-[min(560px,calc(100dvh-4rem))] flex-col gap-4 overflow-hidden sm:max-w-md">
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription className="sr-only">{t('settings:aiDialogDescription')}</DialogDescription>
      </DialogHeader>
      {step.kind === 'pick' && <ProviderPick client={client} providers={catalog.data?.providers ?? []} loading={catalog.isPending}
        onPick={(provider) => setStep({ kind: 'form', provider })} />}
      {step.kind === 'form' && <ConnectionForm client={client} state={state} editing={editing} provider={step.provider}
        onBack={editing === null ? () => setStep({ kind: 'pick' }) : undefined} onCancel={onClose}
        onSaved={(saved) => {
          onState(saved.state)
          if (editing === null && saved.state.defaultModel === null) {
            const connection = saved.state.connections.find((entry) => entry.id === saved.connectionId)
            setStep({ kind: 'model', connectionId: saved.connectionId, name: connection?.name ?? step.provider.name, providerId: step.provider.id, models: saved.models })
          } else onClose()
        }} />}
      {step.kind === 'model' && <ChooseModel client={client} step={step} onState={onState} onClose={onClose} />}
    </DialogContent>
  </Dialog>
}

function ProviderPick({ client, providers, loading, onPick }: {
  client: MailxClient; providers: AiCatalogProvider[]; loading: boolean; onPick: (provider: AiCatalogProvider) => void
}) {
  useTranslation()
  const custom: AiCatalogProvider = { id: CUSTOM_ID, name: t('settings:aiCustom'), protocol: 'openaiCompatible', baseUrl: '', builtIn: false }
  const builtIn = providers.filter((provider) => provider.builtIn)
  const others = providers.filter((provider) => !provider.builtIn && provider.id !== CUSTOM_ID).sort((a, b) => a.name.localeCompare(b.name))
  const item = (provider: AiCatalogProvider) => (
    <CommandItem key={provider.id} value={provider.id} keywords={[provider.name]} onSelect={() => onPick(provider)} className="py-2 text-sm">
      <AiProviderIcon client={client} providerId={provider.id} name={provider.name} />
      <span className="min-w-0 flex-1 truncate">{provider.name}</span>
      {provider.local && <span className="shrink-0 rounded-md bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">{t('settings:aiLocal')}</span>}
    </CommandItem>
  )
  return <Command className="min-h-0 flex-1 rounded-lg ring-1 ring-foreground/10">
    <CommandInput autoFocus placeholder={t('settings:aiSearchProviders')} />
    <CommandList className="max-h-80">
      {loading && <div className="flex justify-center py-6"><Spinner /></div>}
      <CommandGroup>
        {builtIn.map(item)}
        {item(custom)}
        {others.map(item)}
      </CommandGroup>
    </CommandList>
  </Command>
}

function ConnectionForm({ client, state, editing, provider, onBack, onCancel, onSaved }: {
  client: MailxClient; state: AiState; editing: AiConnectionState | null; provider: AiCatalogProvider
  onBack?: () => void; onCancel: () => void; onSaved: (saved: Awaited<ReturnType<MailxClient['aiSaveConnection']>>) => void
}) {
  useTranslation()
  const isCustom = provider.id === CUSTOM_ID
  const proxyAvailable = state.route.kind === 'proxy'
  const [name, setName] = useState(editing?.name ?? (isCustom ? '' : provider.name))
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl)
  const [protocol, setProtocol] = useState<AiProtocol>(provider.protocol)
  const [useProxy, setUseProxy] = useState(editing?.useProxy ?? true)
  const [key, setKey] = useState('')
  const [cleared, setCleared] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const hasKey = editing?.hasKey === true
  const ready = name.trim() !== '' && baseUrl.trim() !== ''
  const protocols: Record<string, string> = { openaiCompatible: t('settings:aiProtocolOpenai'), anthropic: t('settings:aiProtocolAnthropic') }
  const submit = (verify: boolean) => {
    if (!ready || busy) return
    setBusy(true)
    setFailure(null)
    const typed = key.trim()
    client.aiSaveConnection({
      id: editing?.id ?? null, providerId: provider.id, name: name.trim(), protocol, baseUrl: baseUrl.trim(),
      useProxy: proxyAvailable && useProxy, apiKey: cleared ? '' : typed === '' ? null : typed, verify,
    }).then(onSaved, (error: unknown) => { setFailure(errorText(error)); setBusy(false) })
  }
  const onSubmit = (event: FormEvent) => { event.preventDefault(); submit(true) }
  return <form className="flex min-h-0 flex-col gap-4 overflow-y-auto" onSubmit={onSubmit}>
    <Field>
      <FieldLabel htmlFor="ai.name">{t('settings:aiFieldName')}</FieldLabel>
      <Input id="ai.name" value={name} maxLength={64} autoComplete="off" disabled={busy} onChange={(event) => setName(event.target.value)} />
    </Field>
    <Field>
      <FieldLabel htmlFor="ai.key">{t('settings:aiFieldKey')}</FieldLabel>
      <div className="flex items-center gap-2">
        <Input id="ai.key" type="password" autoComplete="off" spellCheck={false} value={key} disabled={busy || cleared}
          placeholder={cleared ? t('settings:aiKeyWillClear') : hasKey ? t('settings:aiKeySaved') : provider.local ? t('settings:aiKeyPlaceholderLocal') : t('settings:aiKeyPlaceholder')}
          onChange={(event) => setKey(event.target.value)} />
        {hasKey && !cleared && key === '' && <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => setCleared(true)}>{t('settings:aiKeyClear')}</Button>}
        {cleared && <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => setCleared(false)}>{t('settings:aiKeyKeep')}</Button>}
      </div>
    </Field>
    <Field>
      <FieldLabel htmlFor="ai.baseUrl">{t('settings:aiFieldUrl')}</FieldLabel>
      <Input id="ai.baseUrl" value={baseUrl} autoComplete="off" spellCheck={false} disabled={busy} placeholder="https://" onChange={(event) => setBaseUrl(event.target.value)} />
    </Field>
    {isCustom && <Field>
      <FieldLabel htmlFor="ai.protocol">{t('settings:aiFieldProtocol')}</FieldLabel>
      <Select value={protocol} items={protocols} disabled={busy} onValueChange={(value: string | null) => { if (value === 'openaiCompatible' || value === 'anthropic') setProtocol(value) }}>
        <SelectTrigger id="ai.protocol" className="w-full"><SelectValue /></SelectTrigger>
        <SelectContent>{Object.entries(protocols).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
      </Select>
    </Field>}
    {proxyAvailable && <Field orientation="horizontal">
      <FieldContent><FieldLabel htmlFor="ai.useProxy">{t('settings:aiUseProxy')}</FieldLabel></FieldContent>
      <Switch id="ai.useProxy" checked={useProxy} disabled={busy} onCheckedChange={setUseProxy} />
    </Field>}
    {failure && <p role="alert" className="text-sm text-destructive">{failure}</p>}
    <div className="flex items-center justify-end gap-2">
      {onBack && <Button type="button" variant="ghost" className="mr-auto" disabled={busy} onClick={onBack}>{t('settings:aiBack')}</Button>}
      {failure === null && <Button type="button" variant="ghost" disabled={busy} onClick={onCancel}>{t('settings:aiCancel')}</Button>}
      {failure !== null && <Button type="button" variant="outline" disabled={busy} onClick={() => submit(false)}>{t('settings:aiSaveAnyway')}</Button>}
      <Button type="submit" disabled={!ready || busy}>
        {busy && <Spinner />}
        {busy ? t('settings:aiConnecting') : t('settings:aiConnect')}
      </Button>
    </div>
  </form>
}

/** After the first provider is connected: its models, or any typed id, become the default model. */
function ChooseModel({ client, step, onState, onClose }: {
  client: MailxClient; step: Extract<Step, { kind: 'model' }>; onState: (state: AiState) => void; onClose: () => void
}) {
  useTranslation()
  const models = useQuery({
    queryKey: aiKeys.models(step.connectionId), queryFn: () => client.aiListModels(step.connectionId),
    initialData: step.models ?? undefined, staleTime: step.models ? Infinity : 0, gcTime: 0,
  })
  const [failure, setFailure] = useState<string | null>(null)
  const group = { id: step.connectionId, name: step.name, providerId: step.providerId, models: models.data, loading: models.isFetching && !models.data, failure: models.isError ? errorText(models.error) : null }
  return <div className="flex min-h-0 flex-1 flex-col gap-3">
    <div className="min-h-0 flex-1 overflow-hidden rounded-lg ring-1 ring-foreground/10">
      <ModelList client={client} groups={[group]} onPick={(model) => {
        setFailure(null)
        client.aiSetDefaultModel(model).then((next) => { onState(next); onClose() }, (error: unknown) => setFailure(errorText(error)))
      }} />
    </div>
    {failure && <p role="alert" className="text-sm text-destructive">{failure}</p>}
    <div className="flex justify-end"><Button variant="ghost" onClick={onClose}>{t('settings:aiSkip')}</Button></div>
  </div>
}
