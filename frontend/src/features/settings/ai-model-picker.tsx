import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Command, CommandGroup, CommandInput, CommandItem, CommandList } from '@/components/ui/command'
import { Spinner } from '@/components/ui/spinner'
import type { AiModel, AiModelRef } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { formatTokens, modelRef } from './ai-models'
import { AiProviderIcon } from './ai-provider-icon'

/** One connection and what is known about its models; a failed listing only affects its own group. */
export type ModelGroup = {
  id: string
  name: string
  providerId: string
  models: AiModel[] | undefined
  loading: boolean
  failure: string | null
}

/** A searchable list of models, grouped by connection. Any typed text can be used as a model id. */
export function ModelList({ client, groups, onPick, leading }: { client: MailxClient; groups: ModelGroup[]; onPick: (model: AiModelRef) => void; leading?: ReactNode }) {
  useTranslation()
  const [search, setSearch] = useState('')
  const typed = search.trim()
  return <Command>
    <CommandInput autoFocus value={search} onValueChange={setSearch} placeholder={t('settings:aiModelSearch')} />
    <CommandList>
      {leading}
      {groups.map((group) => {
        const heading = <span className="inline-flex items-center gap-1.5">
          <AiProviderIcon client={client} providerId={group.providerId} name={group.name} className="size-4 text-[10px]" />
          {group.name}
        </span>
        const exact = group.models?.some((model) => model.id === typed) ?? false
        return <CommandGroup key={group.id} heading={heading} forceMount>
          {group.loading && <div className="flex items-center gap-2 px-2 py-1.5 text-xs text-muted-foreground"><Spinner className="size-3.5" />{t('settings:aiModelLoading')}</div>}
          {group.failure && <p role="alert" className="px-2 py-1.5 text-xs text-destructive">{group.failure}</p>}
          {group.models?.length === 0 && typed === '' && <p className="px-2 py-1.5 text-xs text-muted-foreground">{t('settings:aiModelNone')}</p>}
          {group.models?.map((model) => (
            <CommandItem key={model.id} forceMount={false} value={`${group.id}/${model.id}`} keywords={[model.name, model.id]} onSelect={() => onPick(modelRef(group.id, model))}>
              <span className="min-w-0 flex-1 truncate">{model.name}</span>
              {model.contextLength !== null && <span className="shrink-0 text-muted-foreground">{t('settings:aiContextHint', { size: formatTokens(model.contextLength) })}</span>}
            </CommandItem>
          ))}
          {typed !== '' && !exact && (
            <CommandItem forceMount value={`manual/${group.id}`} onSelect={() => onPick(modelRef(group.id, typed))}>
              {t('settings:aiModelUse', { id: typed })}
            </CommandItem>
          )}
        </CommandGroup>
      })}
    </CommandList>
  </Command>
}
