import { useEffect, useId } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Search01Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import { ProviderIcon } from './provider-badge'
import { filterProviders, PROVIDERS, type MailProvider } from './providers'

type ProviderPickerProps = {
  selected: MailProvider | null
  query: string
  onQueryChange(query: string): void
  onSelect(provider: MailProvider | null): void
  onAdvance(provider: MailProvider): void
  providers?: readonly MailProvider[]
}

/** Native radios provide one tab stop, arrow navigation and Space selection. */
export function ProviderPicker({ selected, query, onQueryChange, onSelect, onAdvance, providers = PROVIDERS }: ProviderPickerProps) {
  useTranslation()
  const id = useId()
  const visible = filterProviders(providers, query)
  const selectionVisible = !selected || visible.some((provider) => provider.id === selected.id)

  // Another window can change the language and therefore the searchable description.
  useEffect(() => {
    if (!selectionVisible) onSelect(null)
  }, [selectionVisible, onSelect])

  function search(value: string) {
    onQueryChange(value)
    if (selected && !filterProviders(providers, value).some((provider) => provider.id === selected.id)) onSelect(null)
  }

  return (
    <>
      <div className="shrink-0 px-6 pb-3">
        <p id={`${id}-label`} className="pb-3 text-sm text-muted-foreground">{t('accounts:chooseProvider')}</p>
        <div className="relative">
          <HugeiconsIcon icon={Search01Icon} aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input type="search" value={query} onChange={(event) => search(event.target.value)}
            aria-label={t('accounts:searchProviders')} placeholder={t('accounts:searchProvidersPlaceholder')}
            className="pl-9" autoComplete="off" autoCapitalize="none" spellCheck={false} />
        </div>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div role="radiogroup" aria-labelledby={`${id}-label`} className="grid gap-1 px-6 pb-6">
          {visible.map((provider, index) => {
            const checked = selected?.id === provider.id
            const rowId = `${id}-${index}`
            return (
              <label key={provider.id ?? 'generic'} onDoubleClick={() => onAdvance(provider)}
                className={cn(
                  'grid min-h-16 cursor-pointer grid-cols-[2rem_minmax(0,1fr)_minmax(0,1.5fr)_1rem] items-center gap-2 rounded-lg px-3 py-2 text-left transition-colors hover:bg-muted/50 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring has-[:focus-visible]:ring-inset sm:grid-cols-[2.5rem_8rem_minmax(0,1fr)_1rem] sm:gap-3',
                  checked && 'bg-primary/5 hover:bg-primary/5',
                )}>
                <input type="radio" name={`${id}-provider`} value={provider.id ?? ''} checked={checked}
                  onChange={() => onSelect(provider)} aria-labelledby={`${rowId}-name`} aria-describedby={`${rowId}-description`}
                  className="sr-only" />
                <ProviderIcon provider={provider} className="size-8 sm:size-10" />
                <span id={`${rowId}-name`} className="truncate text-sm font-medium">{provider.name}</span>
                <span id={`${rowId}-description`} className="truncate text-xs text-muted-foreground">{provider.description}</span>
                <span aria-hidden className="flex size-4 items-center justify-center text-primary">
                  {checked && <HugeiconsIcon icon={Tick02Icon} className="size-4" strokeWidth={3} />}
                </span>
              </label>
            )
          })}
        </div>
        {visible.length === 0 && <p role="status" className="px-6 py-8 text-center text-sm text-muted-foreground">{t('accounts:noProvidersFound')}</p>}
      </ScrollArea>
    </>
  )
}
