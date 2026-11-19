import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Command, CommandGroup, CommandInput, CommandItem, CommandList } from '@/components/ui/command'
import { Spinner } from '@/components/ui/spinner'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import type { AiModel, AiModelRef, AiModelSource } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { formatTokens, modelRef } from './ai-models'
import { AiProviderIcon } from './ai-provider-icon'
import { type ModelPickerConnection, useModelGroups } from './ai-queries'

function groupByOwner(models: AiModel[] | null | undefined) {
  const groups = new Map<string | null, AiModel[]>()
  for (const model of models ?? []) {
    const owner = model.ownedBy ?? null
    const group = groups.get(owner)
    if (group) group.push(model)
    else groups.set(owner, [model])
  }
  return groups
}

/** A picker always chooses a model for an existing connection, regardless of the list source. */
export function ModelList({ client, connections, onPick, leading }: {
  client: MailxClient; connections: ModelPickerConnection[]; onPick: (model: AiModelRef) => void; leading?: ReactNode
}) {
  useTranslation()
  const [source, setSource] = useState<AiModelSource>('provider')
  const [search, setSearch] = useState('')
  const groups = useModelGroups(client, connections, source)
  const typed = search.trim()
  return <Tabs value={source} onValueChange={(value: AiModelSource) => setSource(value)} className="h-full min-h-0 gap-1">
    <TabsList aria-label={t('settings:aiModelSource')} className="mx-2 mt-2 shrink-0 self-start">
      <TabsTrigger value="provider">{t('settings:aiModelSourceProvider')}</TabsTrigger>
      <TabsTrigger value="catalog">{t('settings:aiModelSourceCatalog')}</TabsTrigger>
    </TabsList>
    <TabsContent value={source} className="flex min-h-0 flex-col">
      <Command className="min-h-0 rounded-lg [&_[data-slot=command-input-wrapper]]:shrink-0">
        <CommandInput autoFocus value={search} onValueChange={setSearch} placeholder={t('settings:aiModelSearch')} />
        {source === 'catalog' && groups.some(({ models }) => models?.length) && (
          <p className="shrink-0 px-3 pt-2 text-xs text-muted-foreground">{t('settings:aiModelCatalogUnverified')}</p>
        )}
        <CommandList>
          {leading}
          {groups.map(({ connection, models, loading, failure }) => {
            const heading = (owner: string | null) => <span className="flex items-center gap-1.5">
              <AiProviderIcon client={client} providerId={connection.providerId} name={connection.name} className="size-4 shrink-0 text-[10px]" />
              <span className="truncate" title={owner ? `${connection.name} · ${owner}` : connection.name}>{connection.name}{owner && ` · ${owner}`}</span>
            </span>
            const manual = typed !== '' && !models?.some((model) => model.id === typed)
            const empty = models?.length === 0 && !loading && !failure
            const showConnectionDetails = loading || Boolean(failure) || models === null || empty || manual
            const byOwner = groupByOwner(models)
            if (showConnectionDetails && !byOwner.has(null)) byOwner.set(null, [])
            return Array.from(byOwner, ([owner, entries]) => (
              <CommandGroup key={JSON.stringify([connection.id, owner])} heading={heading(owner)} forceMount={owner === null && showConnectionDetails}>
                {entries.map((model) => (
                  <CommandItem key={model.id} value={`${connection.id}/${model.id}`} keywords={[model.name, model.id, connection.name, ...(owner ? [owner] : [])]}
                    onSelect={() => onPick(modelRef(connection.id, model))}>
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className="truncate" title={model.name}>{model.name}</span>
                      {model.id !== model.name && <span className="truncate text-[11px] text-muted-foreground" title={model.id}>{model.id}</span>}
                    </span>
                    {model.contextLength !== null && <span className="shrink-0 text-muted-foreground">{t('settings:aiContextHint', { size: formatTokens(model.contextLength) })}</span>}
                  </CommandItem>
                ))}
                {owner === null && <>
                  {loading && <div role="status" className="flex items-center gap-2 px-2 py-1.5 text-xs text-muted-foreground"><Spinner aria-hidden="true" className="size-3.5" />{t('settings:aiModelLoading')}</div>}
                  {failure && <p role="alert" className="px-2 py-1.5 text-xs text-destructive">{failure}</p>}
                  {models === null && <p className="px-2 py-1.5 text-xs text-muted-foreground">{t('settings:aiModelListingUnsupported')}</p>}
                  {empty && <p className="px-2 py-1.5 text-xs text-muted-foreground">{t(source === 'catalog' ? 'settings:aiModelCatalogNone' : 'settings:aiModelNone')}</p>}
                  {manual && <CommandItem forceMount value={`manual/${connection.id}`} onSelect={() => onPick(modelRef(connection.id, typed))}>
                    <span className="truncate" title={typed}>{t('settings:aiModelUse', { id: typed })}</span>
                  </CommandItem>}
                </>}
              </CommandGroup>
            ))
          })}
        </CommandList>
      </Command>
    </TabsContent>
  </Tabs>
}
