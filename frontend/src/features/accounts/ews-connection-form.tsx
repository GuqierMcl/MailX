import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Account, ConnectionInput, DraftConnectionVerification, EwsSettingsDiscovery } from '@/generated/accounts'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { FieldDescription, FieldSet } from '@/components/ui/field'
import { SettingsGroup, SettingsPage } from '@/features/settings/settings-layout'
import { clientFailure, MailxClientError, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { changeDraft, clearSecrets, draftToRoute, initialDraft, type AccountDraft } from './account-draft'
import { PasswordField, RouteFields, TextField } from './connection-form'
import { ConnectionVerificationReport } from './connection-verification'

type Props = {
  client: MailxClient
  account: Account | null
  initialDraft?: AccountDraft
  initialError?: ClientFailure | null
  submitLabel: string
  submittingLabel: string
  onSubmit(connection: ConnectionInput, draft: AccountDraft): Promise<void> | void
  onBusyChange(busy: boolean): void
  onDirtyChange(dirty: boolean): void
  onBack?(): void
  onReload?(): void
}

function ewsDraft(account: Account | null): AccountDraft {
  const draft = initialDraft(account)
  const credential = account?.authentication.kind === 'ewsNtlm' ? account.authentication.credential : null
  return {
    ...draft,
    ews: account && 'ews' in account ? { ...account.ews, manual: true } : { url: '', username: '', manual: false },
    sharedPassword: { replace: !credential?.passwordConfigured, value: '' },
  }
}

function failure(field: string, message: string): never {
  throw new MailxClientError({ code: 'validation', field, message })
}

function ewsConnection(draft: AccountDraft, account: Account | null): ConnectionInput {
  const emailAddress = draft.emailAddress.trim()
  if (!/^[^\s@]+@[^\s@]+$/.test(emailAddress)) failure('emailAddress', t('accounts:invalidEmail'))
  const ews = draft.ews
  let url: URL
  try { url = new URL(ews?.url.trim() ?? '') } catch { failure('ews.url', t('accounts:ewsInvalidUrl')) }
  if (url.protocol !== 'https:' || !url.hostname || url.username || url.password || url.search || url.hash) failure('ews.url', t('accounts:ewsInvalidUrl'))
  const username = ews?.username.trim() ?? ''
  if (!username) failure('ews.username', t('accounts:ewsLoginRequired'))
  if (draft.sharedPassword.replace && !draft.sharedPassword.value) failure('sharedPassword', t('accounts:passwordRequired'))
  return {
    backend: 'ews', ews: { url: url.href, username }, emailAddress,
    providerId: account?.providerId ?? 'exchange-self-hosted',
    authentication: { kind: 'password', mode: 'shared', password: draft.sharedPassword.replace ? { action: 'replace', value: draft.sharedPassword.value } : { action: 'keep' } },
    route: draftToRoute(draft),
  }
}

/** An independent EWS path. Address edits never reconfigure another protocol. */
export function EwsConnectionForm({ client, account, initialDraft: resume, initialError = null, submitLabel, submittingLabel, onSubmit, onBusyChange, onDirtyChange, onBack, onReload }: Props) {
  useTranslation()
  const [draft, setDraft] = useState(() => resume ?? ewsDraft(account))
  const [error, setError] = useState<ClientFailure | null>(initialError)
  const [lookup, setLookup] = useState<EwsSettingsDiscovery | null>(null)
  const [finding, setFinding] = useState(false)
  const [saving, setSaving] = useState(false)
  const [report, setReport] = useState<DraftConnectionVerification | null>(null)
  const [testing, setTesting] = useState(false)
  const seq = useRef(0)
  const alive = useRef(true)
  const test = useRef<AbortController | null>(null)
  const submission = useRef(false)
  const credential = account?.authentication.kind === 'ewsNtlm' ? account.authentication.credential : null
  const manual = draft.ews?.manual ?? false
  const oldProxyAuth = account?.route.kind === 'custom' ? account.route.authentication : null

  useEffect(() => {
    alive.current = true
    return () => { alive.current = false; seq.current += 1; test.current?.abort() }
  }, [])

  function invalidate(keepGrant = false) {
    seq.current += 1
    test.current?.abort()
    test.current = null
    setTesting(false)
    setFinding(false)
    setReport(null)
    if (!keepGrant) setLookup(null)
    setError(null)
    onBusyChange(false)
  }
  function change<K extends keyof AccountDraft>(key: K, value: AccountDraft[K]) {
    invalidate(key === 'sharedPassword')
    setDraft((current) => changeDraft(current, key, value))
    onDirtyChange(true)
  }
  function server(key: 'url' | 'username', value: string) {
    if (!manual && key === 'username') {
      invalidate(true)
      setDraft((current) => ({ ...current, ews: { url: current.ews?.url ?? '', username: value, manual: false } }))
      onDirtyChange(true)
      return
    }
    change('ews', { url: draft.ews?.url ?? '', username: draft.ews?.username ?? '', manual, [key]: value })
  }
  function setManual(value: boolean) {
    change('ews', { url: draft.ews?.url ?? '', username: draft.ews?.username || draft.emailAddress.trim(), manual: value })
  }
  async function discover(grant?: EwsSettingsDiscovery) {
    if (submission.current) return
    let route
    const emailAddress = draft.emailAddress.trim()
    try {
      if (!/^[^\s@]+@[^\s@]+$/.test(emailAddress)) failure('emailAddress', t('accounts:invalidEmail'))
      route = draftToRoute(draft)
      if (grant && (!draft.sharedPassword.replace || !draft.sharedPassword.value)) failure('sharedPassword', t('accounts:passwordRequired'))
    } catch (cause) { setError(clientFailure(cause)); return }
    const mine = ++seq.current
    setFinding(true)
    setError(null)
    onBusyChange(true)
    try {
      const result = await client.discoverEwsSettings({ emailAddress, route, targetGrant: grant?.targetGrant ?? null,
        username: grant ? draft.ews?.username.trim() || emailAddress : null,
        password: grant ? draft.sharedPassword.value : null })
      if (!alive.current || mine !== seq.current) return
      setLookup(result)
      if (result.status === 'found' && result.endpoint) {
        setDraft((current) => ({ ...current, ews: { ...result.endpoint!, manual: true } }))
      }
    } catch (cause) {
      if (alive.current && mine === seq.current) setError(clientFailure(cause))
    } finally {
      if (alive.current && mine === seq.current) { setFinding(false); onBusyChange(false) }
    }
  }
  async function testLogin() {
    if (testing || saving) return
    let connection
    try { connection = ewsConnection(draft, account) } catch (cause) { setError(clientFailure(cause)); return }
    const controller = new AbortController()
    test.current = controller
    setTesting(true)
    setError(null)
    setReport(null)
    try {
      const result = await client.verifyDraftConnection(connection, controller.signal)
      if (alive.current && test.current === controller && !controller.signal.aborted) setReport(result)
    } catch (cause) {
      if (alive.current && test.current === controller && !controller.signal.aborted) setError(clientFailure(cause))
    } finally {
      if (alive.current && test.current === controller) { test.current = null; setTesting(false) }
    }
  }
  async function save() {
    if (submission.current) return
    if (!manual) { await discover(); return }
    let connection
    try { connection = ewsConnection(draft, account) } catch (cause) { setError(clientFailure(cause)); return }
    submission.current = true
    setSaving(true)
    onBusyChange(true)
    test.current?.abort()
    const submitted = draft
    setDraft(clearSecrets)
    try { await onSubmit(connection, submitted); onDirtyChange(false) }
    catch (cause) { if (alive.current) setError(clientFailure(cause)) }
    finally { submission.current = false; if (alive.current) { setSaving(false); onBusyChange(false) } }
  }

  return <SettingsPage footer={<>
    <FieldDescription>{saving ? t('accounts:savingSecurely') : t('accounts:credentialsNotDisplayed')}</FieldDescription>
    <div className="flex shrink-0 gap-2">
      {onBack && <Button variant="outline" disabled={saving} onClick={onBack}>{t('accounts:back')}</Button>}
      <Button disabled={saving || finding || !client.available} onClick={() => void save()}>{saving ? submittingLabel : submitLabel}</Button>
    </div>
  </>}>
    <Alert role="status"><AlertTitle>{t('accounts:ewsSetupTitle')}</AlertTitle><AlertDescription>{t('accounts:ewsSetupDescription')}</AlertDescription></Alert>
    {error && <Alert variant="destructive"><AlertTitle>{t('accounts:notSaved')}</AlertTitle><AlertDescription>{errorText(error)}</AlertDescription>
      {onReload && error.code === 'conflict' && <Button variant="outline" onClick={onReload}>{t('accounts:reloadSettings')}</Button>}
    </Alert>}
    {credential?.trust === 'needsConfirmation' && <Alert><AlertDescription>{t('accounts:ewsConnectionUntrusted')}</AlertDescription></Alert>}
    <FieldSet disabled={saving} aria-label={t('accounts:emailAccountSettings')} className="gap-6">
      <SettingsGroup title={t('accounts:emailAddress')}>
        <TextField id="emailAddress" label={t('accounts:emailAddress')} value={draft.emailAddress} onChange={(value) => change('emailAddress', value)} placeholder="you@company.com" inputMode="email" error={error} />
      </SettingsGroup>
      {manual ? <SettingsGroup title={t('accounts:ewsSettingsTitle')} description={t('accounts:ewsNtlmHint')}>
        <TextField id="ews.url" label={t('accounts:ewsUrlLabel')} value={draft.ews?.url ?? ''} onChange={(value) => server('url', value)} placeholder="https://mail.example.com/EWS/Exchange.asmx" description={t('accounts:ewsUrlHint')} error={error} />
        <TextField id="ews.username" label={t('accounts:ewsUsernameLabel')} value={draft.ews?.username ?? ''} onChange={(value) => server('username', value)} description={t('accounts:ewsUsernameHint')} error={error} />
        {!account && <Button variant="ghost" className="w-fit" onClick={() => setManual(false)}>{t('accounts:ewsAutomaticSetup')}</Button>}
      </SettingsGroup> : <SettingsGroup title={t('accounts:ewsSettingsTitle')}>
        <Button variant="outline" className="w-fit" disabled={finding || !client.available} onClick={() => void discover()}>{t(finding ? 'accounts:ewsFindingServer' : 'accounts:ewsFindServer')}</Button>
        <Button variant="ghost" className="w-fit" onClick={() => setManual(true)}>{t('accounts:ewsManualSetup')}</Button>
      </SettingsGroup>}
      {!manual && lookup?.targetGrant && <SettingsGroup title={t('accounts:ewsUsernameLabel')}>
        <TextField id="ews.username" label={t('accounts:ewsUsernameLabel')} value={draft.ews?.username || draft.emailAddress.trim()} onChange={(value) => server('username', value)} description={t('accounts:ewsUsernameHint')} error={error} />
      </SettingsGroup>}
      <SettingsGroup title={t('accounts:credentials')}>
        <PasswordField id="sharedPassword" label={t('accounts:ewsPasswordLabel')} value={draft.sharedPassword} onChange={(value) => change('sharedPassword', value)} canKeep={!!credential?.passwordConfigured} error={error} />
      </SettingsGroup>
      <RouteFields draft={draft} change={change} error={error} oldProxyAuth={oldProxyAuth} />
    </FieldSet>
    {finding && <p role="status" className="text-sm text-muted-foreground">{t('accounts:ewsFindingServer')}</p>}
    {lookup && <Alert role="status"><AlertTitle>{t(`accounts:ewsDiscoveryStatus${lookup.status}`)}</AlertTitle>
      {lookup.source && <AlertDescription>{t('accounts:ewsDiscoverySource', { source: lookup.source })}</AlertDescription>}
      {lookup.targetGrant && <>
        <AlertDescription>{t('accounts:ewsDiscoveryConsent', { target: lookup.source ?? '' })}</AlertDescription>
        <Button className="mt-3 w-fit" variant="outline" disabled={finding || !client.available} onClick={() => void discover(lookup)}>{t('accounts:ewsContinueDiscovery')}</Button>
      </>}
    </Alert>}
    {manual && !account && <SettingsGroup title={t('accounts:testLogin')} description={t('accounts:draftVerificationHint')}>
      <div className="flex gap-2"><Button variant="outline" disabled={testing || saving || !client.available} onClick={() => void testLogin()}>{t(testing ? 'accounts:testingLogin' : 'accounts:testLogin')}</Button>
        {testing && <Button variant="ghost" onClick={() => { test.current?.abort(); test.current = null; setTesting(false) }}>{t('accounts:cancelLoginTest')}</Button>}</div>
      {report && <ConnectionVerificationReport report={report} client={client} providerId="exchange-self-hosted" />}
    </SettingsGroup>}
  </SettingsPage>
}
