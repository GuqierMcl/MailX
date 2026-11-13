import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import type { TranslationKeyState, TranslationProviderId, TranslationProxyPasswordState, TranslationSettings } from '@/generated/accounts'
import { COMMON_LANGUAGES, KEYED_PROVIDERS, KEYLESS_PROVIDERS, languageName, nativeLanguageName, needsKey, TRANSLATION_PROVIDER_NAMES, TRANSLATION_TARGETS } from '@/features/mail/translation/languages'
import type { MailxClient } from '@/lib/mailx-client'
import { translationKeys } from '@/lib/query-client'
import { ProxyPasswordFields, ProxyRows, StoredSecretRow } from './proxy-rows'
import { SettingsDisclosure, SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateTranslation } from './settings-queries'

/** What the core accepts for the Microsoft resource region. */
const REGION = /^[a-z0-9-]{0,40}$/
const NONE = '__none__'
const AUTO = 'auto'

type Update = ReturnType<typeof useUpdateTranslation>

export function TranslationPane({ client }: { client: MailxClient }) {
  useTranslation()
  const settings = useSettings(client)
  const update = useUpdateTranslation(client)
  if (settings.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (settings.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>{t('settings:readFailed')}</AlertTitle><AlertDescription>{errorText(settings.error)}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void settings.refetch()}>{t('settings:retry')}</Button>
  </SettingsPage>
  const translation = settings.data.settings.translation
  return <SettingsPage>
    {update.isError && <Alert variant="destructive"><AlertTitle>{t('settings:saveFailedTitle')}</AlertTitle><AlertDescription>{errorText(update.error)}</AlertDescription></Alert>}
    <ServiceGroup client={client} translation={translation} update={update} />
    <TranslateGroup translation={translation} update={update} />
    <ForeignGroup translation={translation} update={update} />
    <SavedGroup client={client} />
    <AdvancedGroup client={client} translation={translation} update={update} />
  </SettingsPage>
}

function ServiceGroup({ client, translation, update }: { client: MailxClient; translation: TranslationSettings; update: Update }) {
  useTranslation()
  const keys = useQuery({ queryKey: translationKeys.keys(), queryFn: () => client.translationKeys() })
  const { provider } = translation
  const configured = provider !== null && Boolean(keys.data?.some((key) => key.provider === provider && key.configured))
  const keyless = provider !== null && !needsKey(provider)
  const services: Record<string, string> = { [NONE]: t('settings:translationServiceNone'), ...TRANSLATION_PROVIDER_NAMES }
  const option = (value: string) => <SelectItem key={value} value={value}>{services[value]}</SelectItem>
  const description = provider === 'microsoftFree' ? t('settings:translationFreeMicrosoft')
    : provider === 'googleFree' ? t('settings:translationFreeGoogle') : t('settings:translationServiceDescription')
  return <SettingsGroup title={t('settings:translationService')}>
    <SettingsRow label={t('settings:translationServiceLabel')} htmlFor="translation.provider" description={description}>
      <Select value={provider ?? NONE} items={services}
        onValueChange={(value: string | null) => value && update.mutate({ provider: value === NONE ? null : value as TranslationProviderId })}>
        <SelectTrigger id="translation.provider" className="w-56"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectLabel>{t('settings:translationGroupNoSetup')}</SelectLabel>
            {KEYLESS_PROVIDERS.map(option)}
          </SelectGroup>
          <SelectGroup>
            <SelectLabel>{t('settings:translationGroupOwnKey')}</SelectLabel>
            {KEYED_PROVIDERS.map(option)}
          </SelectGroup>
          <SelectGroup>{option(NONE)}</SelectGroup>
        </SelectContent>
      </Select>
    </SettingsRow>
    {provider && !keyless && <>
      <KeyRow key={provider} client={client} provider={provider} configured={configured} />
      {provider === 'microsoft' && <RegionRow region={translation.microsoftRegion} update={update} />}
    </>}
    {provider && <TestRow key={`${provider}:${configured}`} client={client} configured={keyless || configured} keyless={keyless} />}
  </SettingsGroup>
}

/** The key is typed once and kept by the app; it is never read back, so a saved key shows only that it exists. */
function KeyRow({ client, provider, configured }: { client: MailxClient; provider: TranslationProviderId; configured: boolean }) {
  useTranslation()
  const queryClient = useQueryClient()
  const publish = (states: TranslationKeyState[]) => queryClient.setQueryData(translationKeys.keys(), states)
  return <StoredSecretRow id="translation.key" label={t('settings:translationKey')} description={t('settings:translationKeyDescription')}
    placeholder={t('settings:translationKeyPlaceholder')} configured={configured} trim
    onSave={(value) => client.setTranslationKey(provider, value).then(publish)} onRemove={() => client.removeTranslationKey(provider).then(publish)} />
}

function RegionRow({ region, update }: { region: string; update: Update }) {
  useTranslation()
  const [value, setValue] = useState(region)
  const valid = REGION.test(value)
  const commit = () => { if (valid && value !== region) update.mutate({ microsoftRegion: value }) }
  return <SettingsRow label={t('settings:translationRegion')} htmlFor="translation.region" description={t('settings:translationRegionDescription')}>
    <div className="flex flex-col items-end gap-1">
      <Input id="translation.region" className="w-48" value={value} placeholder="eastasia" autoComplete="off" spellCheck={false} aria-invalid={!valid || undefined}
        onChange={(event) => setValue(event.target.value)} onBlur={commit} />
      {!valid && <p role="alert" className="max-w-56 text-right text-xs text-destructive">{t('settings:translationRegionInvalid')}</p>}
    </div>
  </SettingsRow>
}

type TestState = { kind: 'idle' } | { kind: 'running' } | { kind: 'works'; sample: string } | { kind: 'failed'; reason: string }

function TestRow({ client, configured, keyless }: { client: MailxClient; configured: boolean; keyless: boolean }) {
  useTranslation()
  const [state, setState] = useState<TestState>({ kind: 'idle' })
  const run = () => {
    setState({ kind: 'running' })
    client.testTranslation().then(
      ({ translated }) => setState({ kind: 'works', sample: translated }),
      (error: unknown) => setState({ kind: 'failed', reason: errorText(error) }),
    )
  }
  return <SettingsRow label={t('settings:translationTest')} description={keyless ? t('settings:translationTestDescriptionFree') : t('settings:translationTestDescription')}>
    {state.kind === 'works' && <span role="status" className="max-w-56 truncate text-xs text-muted-foreground" title={state.sample}>{t('settings:translationTestWorks', { sample: state.sample })}</span>}
    {state.kind === 'failed' && <span role="alert" className="max-w-56 text-right text-xs text-destructive">{state.reason}</span>}
    <Button variant="outline" size="sm" disabled={!configured || state.kind === 'running'} onClick={run}>
      {state.kind === 'running' ? t('settings:translationTestRunning') : t('settings:translationTestRun')}
    </Button>
  </SettingsRow>
}

/** The connection is the only advanced setting for now; most people never open this, so it says where it points while folded. */
function AdvancedGroup({ client, translation, update }: { client: MailxClient; translation: TranslationSettings; update: Update }) {
  useTranslation()
  const { route } = translation
  const summary = route.kind === 'proxy' ? t('settings:translationAdvancedProxy', { host: route.host, port: route.port }) : t('settings:translationNetworkDirect')
  return <SettingsDisclosure title={t('settings:translationAdvanced')} summary={summary}>
    <ProxyRows idPrefix="translation" route={route} networkDescription={t('settings:translationNetworkDescription')}
      onRouteChange={(next) => update.mutate({ route: next })} password={(hasUsername) => <ProxyPasswordRow client={client} hasUsername={hasUsername} />} />
  </SettingsDisclosure>
}

/** Stored apart from the settings and never read back; without a username it would not be used, so it cannot be set. */
function ProxyPasswordRow({ client, hasUsername }: { client: MailxClient; hasUsername: boolean }) {
  useTranslation()
  const queryClient = useQueryClient()
  const state = useQuery({ queryKey: translationKeys.proxyPassword(), queryFn: () => client.translationProxyPassword() })
  const publish = (next: TranslationProxyPasswordState) => queryClient.setQueryData(translationKeys.proxyPassword(), next)
  return <ProxyPasswordFields id="translation.proxyPassword" hasUsername={hasUsername} configured={state.data?.configured ?? false}
    onSave={(value) => client.setTranslationProxyPassword(value).then(publish)} onRemove={() => client.removeTranslationProxyPassword().then(publish)} />
}

function TranslateGroup({ translation, update }: { translation: TranslationSettings; update: Update }) {
  useTranslation()
  const targets: Record<string, string> = { [AUTO]: t('settings:translationTargetFollow'), ...Object.fromEntries(TRANSLATION_TARGETS.map((code) => [code, nativeLanguageName(code)])) }
  const value = translation.targetLanguage in targets ? translation.targetLanguage : AUTO
  return <SettingsGroup title={t('settings:translationGroupTranslate')}>
    <SettingsRow label={t('settings:translationTarget')} htmlFor="translation.target">
      <Select value={value} items={targets} onValueChange={(next: string | null) => next && update.mutate({ targetLanguage: next })}>
        <SelectTrigger id="translation.target" className="w-56"><SelectValue /></SelectTrigger>
        <SelectContent className="max-h-72">{Object.entries(targets).map(([code, label]) => <SelectItem key={code} value={code}>{label}</SelectItem>)}</SelectContent>
      </Select>
    </SettingsRow>
    <SettingsRow label={t('settings:translationQuotes')} htmlFor="translation.quotes">
      <Switch id="translation.quotes" checked={translation.includeQuotes} onCheckedChange={(includeQuotes) => update.mutate({ includeQuotes })} />
    </SettingsRow>
  </SettingsGroup>
}

function ForeignGroup({ translation, update }: { translation: TranslationSettings; update: Update }) {
  useTranslation()
  const { neverSuggest, alwaysTranslate } = translation
  // A language is in at most one list: adding it to one takes it out of the other.
  const edit = (list: 'neverSuggest' | 'alwaysTranslate', code: string, add: boolean) => {
    const other = list === 'neverSuggest' ? 'alwaysTranslate' : 'neverSuggest'
    const mine = translation[list].filter((entry) => entry !== code)
    update.mutate({ [list]: add ? [...mine, code] : mine, [other]: translation[other].filter((entry) => entry !== code) })
  }
  return <SettingsGroup title={t('settings:translationGroupForeign')}>
    <SettingsRow label={t('settings:translationSuggest')} htmlFor="translation.suggest" description={t('settings:translationSuggestDescription')}>
      <Switch id="translation.suggest" checked={translation.suggest} onCheckedChange={(suggest) => update.mutate({ suggest })} />
    </SettingsRow>
    <LanguageList id="translation.never" label={t('settings:translationNever')} description={t('settings:translationNeverDescription')}
      codes={neverSuggest} taken={alwaysTranslate} onChange={(code, add) => edit('neverSuggest', code, add)} />
    <LanguageList id="translation.always" label={t('settings:translationAlways')} description={t('settings:translationAlwaysDescription')}
      codes={alwaysTranslate} taken={neverSuggest} onChange={(code, add) => edit('alwaysTranslate', code, add)} />
  </SettingsGroup>
}

function LanguageList({ id, label, description, codes, taken, onChange }: {
  id: string; label: string; description: string; codes: string[]; taken: string[]; onChange: (code: string, add: boolean) => void
}) {
  useTranslation()
  const choices = COMMON_LANGUAGES.filter((code) => !codes.includes(code) && !taken.includes(code))
  const items = Object.fromEntries(choices.map((code) => [code, languageName(code)]))
  return <SettingsRow label={label} description={description}>
    <div className="flex max-w-72 flex-wrap items-center gap-1.5 @md/field-group:justify-end">
      {codes.map((code) => (
        <span key={code} className="inline-flex h-6 items-center gap-0.5 rounded-md bg-muted pl-2 text-xs">
          {languageName(code)}
          <Button type="button" variant="ghost" size="icon-xs" className="size-5" aria-label={t('settings:translationRemoveLanguage', { language: languageName(code) })}
            onClick={() => onChange(code, false)}>×</Button>
        </span>
      ))}
      {choices.length > 0 && (
        <Select value={null} items={items} onValueChange={(code: string | null) => code && onChange(code, true)}>
          <SelectTrigger aria-label={`${label}：${t('settings:translationAddLanguage')}`} id={id} size="sm" className="w-36"><SelectValue placeholder={t('settings:translationAddLanguage')} /></SelectTrigger>
          <SelectContent className="max-h-72">{choices.map((code) => <SelectItem key={code} value={code}>{items[code]}</SelectItem>)}</SelectContent>
        </Select>
      )}
    </div>
  </SettingsRow>
}

function SavedGroup({ client }: { client: MailxClient }) {
  useTranslation()
  const queryClient = useQueryClient()
  const stats = useQuery({ queryKey: translationKeys.cache(), queryFn: () => client.translationCacheStats() })
  const [clearing, setClearing] = useState(false)
  const [failed, setFailed] = useState(false)
  const clear = () => {
    setClearing(true)
    setFailed(false)
    client.clearTranslations().then(
      () => queryClient.invalidateQueries({ queryKey: translationKeys.cache() }),
      () => setFailed(true),
    ).finally(() => setClearing(false))
  }
  const count = stats.data?.count ?? 0
  const megabytes = (stats.data?.bytes ?? 0) / (1024 * 1024)
  const summary = count === 0 ? t('settings:translationCacheEmpty')
    : t('settings:translationCacheCount', { count, size: megabytes < 0.05 ? t('settings:translationCacheTiny') : `${megabytes.toFixed(1)} MB` })
  return <SettingsGroup title={t('settings:translationSaved')}>
    <SettingsRow label={stats.isSuccess ? summary : t('settings:translationSaved')} description={t('settings:translationSavedDescription')}>
      {failed && <span role="alert" className="text-xs text-destructive">{t('settings:translationCacheClearFailed')}</span>}
      <Button variant="outline" size="sm" disabled={clearing || count === 0} onClick={clear}>{t('settings:translationCacheClear')}</Button>
    </SettingsRow>
  </SettingsGroup>
}
