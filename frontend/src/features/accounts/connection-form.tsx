import { errorText, type ErrorContext } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import type {
  Account, AuthenticationMode, ConnectionInput, DiscoveredMailEndpoint, DiscoverMailSettingsRequest,
  DraftConnectionVerification, EwsConnectionConfig, MailSettingsDiscovery, MailSettingsDiscoveryStatus,
  RouteInput, TlsMode,
} from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { SettingsGroup, SettingsPage } from '@/features/settings/settings-layout'
import { providerCredentialText, providerEmailRequired, providerOf, type MailProvider } from './providers'
import { changeDraft, clearSecrets, draftToConnection, draftToRoute, initialDraft, type AccountDraft, type ConnectionDraft, type SecretDraft } from './account-draft'
import { ConnectionVerificationReport } from './connection-verification'

type FormProps = {
  /** The saved account whose connection is edited; `null` for a new one. */
  account: Account | null
  provider?: MailProvider
  onOpenHelp?(): Promise<void>
  /** Adds optional testing of the unsaved connection in the add-account wizard. */
  verificationClient?: MailxClient
  /** Read-only server lookup used by the manual IMAP/SMTP path, before asking for servers. */
  discoveryClient?: MailxClient
  /** Invalidates reports when the containing wizard closes or asks to leave. */
  verificationResetKey?: string
  /** Where to resume, e.g. coming back to this step of the add-account wizard. */
  initialDraft?: AccountDraft
  /** A failure to show on arrival, e.g. a later step's save rejected this connection. */
  initialError?: ClientFailure | null
  /** Shown above the fields. */
  intro: { title: string; description: ReactNode }
  submitLabel: string
  submittingLabel: string
  /**
   * Receives the validated connection and the draft it came from (with its
   * secrets). Throw to report a failure; the form's secret inputs are cleared
   * once a submission begins either way.
   */
  onSubmit(connection: ConnectionInput, draft: AccountDraft): Promise<void> | void
  onBusyChange(busy: boolean): void
  onDirtyChange(dirty: boolean): void
  /** Offered after a conflict or an uncertain outcome. */
  onReload?(): void
  /** Disables submission, e.g. when no host backend is available. */
  unavailable?: boolean
  /** Adds a back button beside submit, e.g. in a wizard. */
  onBack?(): void
  /**
   * Generic accounts open on a sign-in card: the address and a password, then
   * either "sign in" (detect, verify with the recommended protocol, submit) or
   * "manual setup" (the full form below, prefilled by the same lookup).
   */
  signInFirst?: boolean
}

type InputFieldProps = {
  id: string
  label: string
  value: string
  onChange(value: string): void
  placeholder?: string
  description?: string
  inputMode?: 'numeric' | 'email' | 'text'
  error?: ClientFailure | null
  autoComplete?: string
  errorContext?: ErrorContext
}

type LoginTestState = (
  | { kind: 'idle' | 'checking' | 'cancelled' }
  | { kind: 'ready'; report: DraftConnectionVerification }
) & { scopeKey: string }

/** What the sign-in card is doing right now. */
type SignInState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'report'; report: DraftConnectionVerification }
  | { kind: 'failed' }

type DiscoveryState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'found'; source: string | null }
  | { kind: 'ews'; source: string | null }
  | { kind: 'needsCredentials' }
  | { kind: 'failed'; status: MailSettingsDiscoveryStatus }
  | { kind: 'manual' }

/** Editing any of these changes which server is looked up, so a result is stale. */
const DISCOVERY_INPUT_KEYS: (keyof AccountDraft)[] = [
  'emailAddress', 'routeKind', 'proxyKind', 'proxyHost', 'proxyPort', 'proxyAuth', 'proxyUsername', 'imageProxyDns',
]

/** One plain sentence for every status a lookup can report. */
const DISCOVERY_STATUS_KEYS = {
  found: 'accounts:autodiscoverStatusFound',
  needsCredentials: 'accounts:autodiscoverStatusNeedsCredentials',
  rejected: 'accounts:autodiscoverStatusRejected',
  notFound: 'accounts:autodiscoverStatusNotFound',
  network: 'accounts:autodiscoverStatusNetwork',
  proxy: 'accounts:autodiscoverStatusProxy',
  tls: 'accounts:autodiscoverStatusTls',
  timeout: 'accounts:autodiscoverStatusTimeout',
  unsupported: 'accounts:autodiscoverStatusUnsupported',
} as const satisfies Record<MailSettingsDiscoveryStatus, string>

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+$/
const DISCOVERY_DELAY_MS = 300

function endpointDraft(endpoint: DiscoveredMailEndpoint): ConnectionDraft {
  // A null username keeps the form's default (the mailbox address).
  return { host: endpoint.host, port: String(endpoint.port), tls: endpoint.tls, username: endpoint.username ?? '' }
}

/** EWS accounts still keep IMAP/SMTP placeholder rows; take their host from the URL. */
function ewsHost(url: string, emailAddress: string): string {
  const fallback = emailAddress.split('@')[1] ?? 'localhost'
  try {
    return new URL(url).hostname || fallback
  } catch {
    return fallback
  }
}

function DiscoveryPanel({ state, source, emailEntered, saving, onRetry }: {
  state: DiscoveryState
  source: string | null
  emailEntered: boolean
  saving: boolean
  onRetry(): void
}) {
  useTranslation()
  const checking = state.kind === 'checking'
  let title = t('accounts:autodiscoverPrompt')
  let detail: string | null = null
  if (checking) title = t('accounts:autodiscoverChecking')
  else if (state.kind === 'found') title = t('accounts:autodiscoverFound')
  else if (state.kind === 'manual') title = t('accounts:autodiscoverManual')
  else if (state.kind === 'needsCredentials') {
    title = t(DISCOVERY_STATUS_KEYS.needsCredentials)
    detail = t('accounts:autodiscoverNeedsCredentialsHint')
  } else if (state.kind === 'failed') {
    title = t(DISCOVERY_STATUS_KEYS[state.status])
    detail = t('accounts:autodiscoverFailedHint')
  }
  return (
    <SettingsGroup title={t('accounts:autodiscoverTitle')}>
      <Alert role="status">
        <AlertTitle>{title}</AlertTitle>
        {detail && <AlertDescription>{detail}</AlertDescription>}
      </Alert>
      {source && <FieldDescription>{t('accounts:autodiscoverSource', { source })}</FieldDescription>}
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" disabled={saving || checking || !emailEntered} onClick={onRetry}>
          {t(state.kind === 'needsCredentials' ? 'accounts:autodiscoverRetryWithPassword' : 'accounts:autodiscoverRetry')}
        </Button>
      </div>
    </SettingsGroup>
  )
}

function EwsFields({ ews, source, saving, onManual, onChange }: {
  ews: EwsConnectionConfig
  source: string | null
  saving: boolean
  onManual(): void
  onChange(value: EwsConnectionConfig): void
}) {
  useTranslation()
  return (
    <SettingsGroup title={t('accounts:ewsSettingsTitle')} description={t('accounts:ewsDescription')}>
      {source && <FieldDescription>{t('accounts:autodiscoverSource', { source })}</FieldDescription>}
      <TextField id="ews.url" label={t('accounts:ewsUrlLabel')} value={ews.url} placeholder="https://mail.example.com/EWS/Exchange.asmx"
        description={t('accounts:ewsUrlHint')} onChange={(url) => onChange({ ...ews, url: url.trim() })} />
      <TextField id="ews.username" label={t('accounts:ewsUsernameLabel')} value={ews.username} placeholder="DOMAIN\\user"
        description={t('accounts:ewsUsernameHint')} onChange={(username) => onChange({ ...ews, username })} />
      <Field>
        <FieldLabel htmlFor="ews.auth">{t('accounts:ewsAuthLabel')}</FieldLabel>
        <Select value={ews.auth} onValueChange={(auth) => { if (auth) onChange({ ...ews, auth }) }}
          items={{ ntlm: 'NTLM', basic: 'Basic' }}>
          <SelectTrigger id="ews.auth" className="w-full" aria-label={t('accounts:ewsAuthLabel')}><SelectValue /></SelectTrigger>
          <SelectContent><SelectItem value="ntlm">NTLM</SelectItem><SelectItem value="basic">Basic</SelectItem></SelectContent>
        </Select>
        <FieldDescription>{t('accounts:ewsAuthHint')}</FieldDescription>
      </Field>
      <TextField id="ews.certPrincipalName" label={t('accounts:ewsCertPrincipalLabel')} value={ews.certPrincipalName ?? ''}
        placeholder="msstd:mail.example.com" description={t('accounts:ewsCertPrincipalHint')}
        onChange={(certPrincipalName) => onChange({ ...ews, certPrincipalName: certPrincipalName.trim() || null })} />
      <Button variant="outline" className="w-fit" disabled={saving} onClick={onManual}>{t('accounts:ewsSwitchToManual')}</Button>
    </SettingsGroup>
  )
}

export function TextField({ id, label, value, onChange, placeholder, description, inputMode, error, autoComplete = 'off', errorContext }: InputFieldProps) {
  const invalid = error?.field === id
  const describedBy = [description && `${id}-description`, invalid && `${id}-error`].filter(Boolean).join(' ') || undefined
  return (
    <Field data-invalid={invalid}>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input id={id} value={value} onChange={(event) => onChange(event.target.value)} placeholder={placeholder}
        inputMode={inputMode} autoComplete={autoComplete} autoCapitalize="none" spellCheck={false}
        aria-invalid={invalid} aria-describedby={describedBy} />
      {description && <FieldDescription id={`${id}-description`}>{description}</FieldDescription>}
      {invalid && <FieldError id={`${id}-error`}>{errorText(error, errorContext)}</FieldError>}
    </Field>
  )
}

function ConnectionFields({ protocol, value, emailAddress, onChange, error }: {
  protocol: 'imap' | 'smtp'
  value: ConnectionDraft
  emailAddress: string
  onChange(value: ConnectionDraft): void
  error: ClientFailure | null
}) {
  useTranslation()
  const isImap = protocol === 'imap'
  const name = protocol.toUpperCase()
  return (
    <SettingsGroup title={isImap ? t('accounts:incomingServer') : t('accounts:outgoingServer')} badge={<Badge variant="secondary">{name}</Badge>}
      description={isImap ? t('accounts:incomingParametersHint') : t('accounts:separateServersHint')}>
        <TextField id={`${protocol}.host`} label={t('accounts:serverHost', { name })} value={value.host} error={error}
          placeholder={isImap ? 'imap.example.com' : 'smtp.example.com'} onChange={(host) => onChange({ ...value, host })} />
        <div className="grid grid-cols-2 gap-4">
          <TextField id={`${protocol}.port`} label={t('accounts:serverPort', { name })} value={value.port} inputMode="numeric" error={error}
            onChange={(port) => onChange({ ...value, port })} />
          <Field>
            <FieldLabel htmlFor={`${protocol}.tls`}>{t('accounts:connectionSecurity')}</FieldLabel>
            <Select value={value.tls} onValueChange={(tls: TlsMode | null) => { if (tls) onChange({ ...value, tls }) }}
              items={{ implicit: 'TLS', startTls: 'STARTTLS' }}>
              <SelectTrigger id={`${protocol}.tls`} className="w-full" aria-label={t('accounts:serverSecurity', { name })}><SelectValue /></SelectTrigger>
              <SelectContent><SelectItem value="implicit">TLS</SelectItem><SelectItem value="startTls">STARTTLS</SelectItem></SelectContent>
            </Select>
          </Field>
        </div>
        <TextField id={`${protocol}.username`} label={t('accounts:serverUsername', { name })} value={value.username}
          placeholder={emailAddress.trim() || t('accounts:usernameDefaultPlaceholder')} description={t('accounts:usernameDefaultHint')}
          error={error} onChange={(username) => onChange({ ...value, username })} />
    </SettingsGroup>
  )
}

function PasswordField({ id, label, value, onChange, canKeep, error, placeholder }: {
  id: string
  label: string
  value: SecretDraft
  onChange(value: SecretDraft): void
  canKeep: boolean
  placeholder?: string
  error: ClientFailure | null
}) {
  useTranslation()
  const invalid = error?.field === id
  return (
    <FieldGroup>
      {canKeep && (
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel htmlFor={`${id}-replace`}>{t('accounts:replaceCredential', { label })}</FieldLabel>
            <FieldDescription>{value.replace ? t('accounts:useNewCredentials') : t('accounts:keepSavedCredentials')}</FieldDescription>
          </FieldContent>
          <Switch id={`${id}-replace`} checked={value.replace} onCheckedChange={(replace) => onChange({ replace, value: '' })} />
        </Field>
      )}
      {value.replace && (
        <Field data-invalid={invalid}>
          <FieldLabel htmlFor={id}>{label}</FieldLabel>
          <Input id={id} type="password" value={value.value} onChange={(event) => onChange({ ...value, value: event.target.value })}
            autoComplete="new-password" autoCapitalize="none" spellCheck={false} placeholder={placeholder ?? t('accounts:enterPassword')}
            aria-invalid={invalid} aria-describedby={invalid ? `${id}-error` : undefined} />
          {invalid && <FieldError id={`${id}-error`}>{errorText(error)}</FieldError>}
        </Field>
      )}
    </FieldGroup>
  )
}

/**
 * How MailX reaches the provider: direct, the app default or a proxy. Shared by
 * the connection form and the browser sign-in step, which has no servers or
 * passwords to ask for.
 */
export function RouteFields({ draft, change, error, oldProxyAuth }: {
  draft: AccountDraft
  change<K extends keyof AccountDraft>(key: K, value: AccountDraft[K]): void
  error: ClientFailure | null
  /** The saved proxy login when editing; `null` for a new account. */
  oldProxyAuth: { passwordConfigured: boolean } | null
}) {
  useTranslation()
  return (<SettingsGroup title={t('accounts:networkRoute')} description={t('accounts:routeDescription')}>
              <Field>
                <FieldLabel htmlFor="routeKind">{t('accounts:connectionMode')}</FieldLabel>
                <Select value={draft.routeKind} onValueChange={(value) => {
                  if (!value) return
                  change('routeKind', value)
                  if (value !== 'custom') change('proxyPassword', { ...draft.proxyPassword, value: '' })
                }}
                  items={{ direct: t('accounts:directConnection'), inherit: t('accounts:appDefault'), custom: t('accounts:customProxy') }}>
                  <SelectTrigger id="routeKind" className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent><SelectItem value="direct">{t('accounts:directConnection')}</SelectItem><SelectItem value="inherit">{t('accounts:appDefault')}</SelectItem><SelectItem value="custom">{t('accounts:customProxy')}</SelectItem></SelectContent>
                </Select>
                <FieldDescription>{draft.routeKind === 'inherit' ? t('accounts:appDefaultHint') : draft.routeKind === 'direct' ? t('accounts:directConnectionHint') : t('accounts:proxyNoFallbackHint')}</FieldDescription>
              </Field>
              {draft.routeKind === 'custom' && <>
                <div className="grid grid-cols-[1fr_1.5fr_0.75fr] gap-4">
                  <Field>
                    <FieldLabel htmlFor="proxyKind">{t('accounts:proxyType')}</FieldLabel>
                    <Select value={draft.proxyKind} onValueChange={(value) => { if (value) change('proxyKind', value) }} items={{ socks5: 'SOCKS5', httpConnect: 'HTTP CONNECT' }}>
                      <SelectTrigger id="proxyKind" className="w-full"><SelectValue /></SelectTrigger>
                      <SelectContent><SelectItem value="socks5">SOCKS5</SelectItem><SelectItem value="httpConnect">HTTP CONNECT</SelectItem></SelectContent>
                    </Select>
                  </Field>
                  <TextField id="proxyHost" label={t('accounts:proxyHost')} value={draft.proxyHost} onChange={(value) => change('proxyHost', value)} placeholder="127.0.0.1" error={error} />
                  <TextField id="proxyPort" label={t('accounts:proxyPort')} value={draft.proxyPort} onChange={(value) => change('proxyPort', value)} inputMode="numeric" error={error} />
                </div>
                <Field orientation="horizontal">
                  <FieldContent><FieldLabel htmlFor="proxyAuth">{t('accounts:proxyAuthentication')}</FieldLabel><FieldDescription>{t('accounts:proxyAuthenticationHint')}</FieldDescription></FieldContent>
                  <Switch id="proxyAuth" checked={draft.proxyAuth} onCheckedChange={(value) => { change('proxyAuth', value); change('proxyPassword', { replace: !oldProxyAuth, value: '' }) }} />
                </Field>
                {draft.proxyAuth && <>
                  <TextField id="proxyUsername" label={t('accounts:proxyUsername')} value={draft.proxyUsername} onChange={(value) => change('proxyUsername', value)} error={error} />
                  <PasswordField id="proxyPassword" label={t('accounts:proxyPassword')} value={draft.proxyPassword} onChange={(value) => change('proxyPassword', value)} canKeep={!!oldProxyAuth?.passwordConfigured} error={error} placeholder={t('accounts:enterProxyPassword')} />
                </>}
                <Field orientation="horizontal">
                  <FieldContent>
                    <FieldLabel htmlFor="imageProxyDns">{t('accounts:proxyHttpsImages')}</FieldLabel>
                    <FieldDescription>{t('accounts:proxyHttpsImagesHint')}</FieldDescription>
                  </FieldContent>
                  <Switch id="imageProxyDns" checked={draft.imageProxyDns} onCheckedChange={(value) => change('imageProxyDns', value)} />
                </Field>
              </>}
          </SettingsGroup>)
}

/**
 * How MailX reaches a mailbox: address, servers, credentials and route. Shared
 * by the add-account wizard and the "edit connection" dialog; the caller
 * decides what submitting means.
 */
export function ConnectionForm({
  account, provider: selectedProvider, onOpenHelp, initialDraft: resume, initialError = null, intro, submitLabel, submittingLabel,
  onSubmit, onBusyChange, onDirtyChange, onReload, unavailable, onBack, verificationClient, discoveryClient, verificationResetKey = '',
  signInFirst = false,
}: FormProps) {
  useTranslation()
  const provider = selectedProvider ?? providerOf(account?.providerId ?? null)
  const setup = provider.setup
  const credentialText = providerCredentialText(setup)
  const errorContext = !account && provider.preset ? { emailAddress: providerEmailRequired(provider) } : undefined
  const [draft, setDraft] = useState(() => resume ?? initialDraft(account, provider))
  const [advanced, setAdvanced] = useState(!!account || !!resume)
  const [helpFailed, setHelpFailed] = useState(false)
  const [error, setError] = useState<ClientFailure | null>(initialError)
  const [testError, setTestError] = useState(false)
  const [saving, setSaving] = useState(false)
  const pending = useRef(false)
  const testPending = useRef<AbortController | null>(null)
  const [loginTest, setLoginTest] = useState<LoginTestState>({ kind: 'idle', scopeKey: verificationResetKey })
  const currentTest = loginTest.scopeKey === verificationResetKey ? loginTest : { kind: 'idle' as const }
  useEffect(() => () => {
    testPending.current?.abort()
    testPending.current = null
  }, [verificationResetKey])
  const discoveryEnabled = !setup && !!discoveryClient
  const autoDiscover = discoveryEnabled && !account
  const [discovery, setDiscovery] = useState<DiscoveryState>({ kind: 'idle' })
  // The sign-in card only exists for a brand-new generic account: an edit form
  // never re-detects on its own and a preset provider already knows its servers.
  const [phase, setPhase] = useState<'signin' | 'form'>(signInFirst && !account && !resume ? 'signin' : 'form')
  const [signInState, setSignInState] = useState<SignInState>({ kind: 'idle' })
  const signingIn = useRef(false)
  const discoverySeq = useRef(0)
  const mounted = useRef(true)
  const draftRef = useRef(draft)
  const discoverRef = useRef<() => Promise<void>>(async () => undefined)
  const discoveryKey = JSON.stringify([
    draft.emailAddress.trim(), draft.routeKind, draft.proxyKind, draft.proxyHost,
    draft.proxyPort, draft.proxyAuth, draft.proxyUsername, draft.imageProxyDns,
  ])
  // A lookup has no abort signal: a sequence number and a mounted flag keep a
  // late answer from overwriting newer input or an unmounted form.
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      discoverySeq.current += 1
    }
  }, [])
  // Keep the latest draft and lookup reachable from the debounced timer.
  useEffect(() => {
    draftRef.current = draft
    discoverRef.current = discover
  })
  useEffect(() => {
    if (!autoDiscover || !EMAIL_PATTERN.test(draftRef.current.emailAddress.trim())) return
    const timer = setTimeout(() => { void discoverRef.current() }, DISCOVERY_DELAY_MS)
    return () => clearTimeout(timer)
  }, [discoveryKey, autoDiscover])
  const password = account?.authentication.kind === 'password' ? account.authentication : null
  const oauth = account?.authentication.kind === 'oauth2'
  const sameMode = password?.mode === draft.mode
  const oldProxyAuth = account?.route.kind === 'custom' ? account.route.authentication : null
  const needsTrust = account && (account.authentication.imap.trust === 'needsConfirmation'
    || account.authentication.smtp.trust === 'needsConfirmation'
    || oldProxyAuth?.trust === 'needsConfirmation')

  function resetTest() {
    testPending.current?.abort()
    testPending.current = null
    setLoginTest({ kind: 'idle', scopeKey: verificationResetKey })
  }

  function change<K extends keyof AccountDraft>(key: K, value: AccountDraft[K]) {
    resetTest()
    const resetsDiscovery = discoveryEnabled && DISCOVERY_INPUT_KEYS.includes(key)
    if (resetsDiscovery) {
      discoverySeq.current += 1
      setDiscovery({ kind: 'idle' })
    }
    setDraft((current) => {
      const changed = changeDraft(current, key, value)
      return resetsDiscovery ? { ...changed, ews: null } : changed
    })
    onDirtyChange(true)
    setError(null)
  }

  function changeMode(mode: AuthenticationMode) {
    resetTest()
    setDraft((current) => ({
      ...current, mode,
      sharedPassword: { replace: true, value: '' },
      imapPassword: { replace: true, value: '' },
      smtpPassword: { replace: true, value: '' },
    }))
    onDirtyChange(true)
    setError(null)
  }

  function showFailure(failure: ClientFailure) {
    setError(failure)
    const field = failure.field
    if (field && !['emailAddress', 'sharedPassword', 'imapPassword', 'smtpPassword'].includes(field)) setAdvanced(true)
    if (field) requestAnimationFrame(() => document.getElementById(field)?.focus())
  }

  function currentDiscoveryPassword(): string | null {
    const current = draftRef.current
    const secret = current.mode === 'shared' ? current.sharedPassword : current.imapPassword
    return secret.replace && secret.value ? secret.value : null
  }

  /**
   * Pure: the draft and lookup state a discovery answer produces. Returning
   * the next draft (instead of only scheduling it) lets the sign-in path read
   * what it just applied without waiting for React to commit.
   */
  function appliedDiscovery(value: AccountDraft, result: MailSettingsDiscovery, hadPassword: boolean, emailAddress: string): { draft: AccountDraft; discovery: DiscoveryState } {
    const ews = result.ews
    if (result.status === 'found' && ews) {
      const login = ews.username ?? emailAddress
      const placeholder: ConnectionDraft = { host: ewsHost(ews.url, emailAddress), port: '443', tls: 'implicit', username: login }
      return {
        draft: {
          ...value,
          ews: { url: ews.url, auth: ews.auth, username: login, certPrincipalName: ews.certPrincipalName },
          imap: { ...placeholder }, smtp: { ...placeholder },
        },
        discovery: { kind: 'ews', source: result.source },
      }
    }
    if (result.status === 'found' && (result.imap || result.smtp)) {
      return {
        draft: {
          ...value, ews: null,
          imap: result.imap ? endpointDraft(result.imap) : value.imap,
          smtp: result.smtp ? endpointDraft(result.smtp) : value.smtp,
        },
        discovery: { kind: 'found', source: result.source },
      }
    }
    if (result.status === 'needsCredentials' && !hadPassword) {
      return { draft: { ...value, ews: null }, discovery: { kind: 'needsCredentials' } }
    }
    return { draft: { ...value, ews: null }, discovery: { kind: 'failed', status: result.status } }
  }

  async function discover() {
    if (!discoveryClient || !discoveryEnabled) return
    const current = draftRef.current
    const emailAddress = current.emailAddress.trim()
    if (!EMAIL_PATTERN.test(emailAddress)) return
    let route: RouteInput
    try {
      route = draftToRoute(current)
    } catch {
      // A half-entered proxy must not block the lookup; the form validates it on save.
      route = { kind: current.routeKind === 'inherit' ? 'inherit' : 'direct' }
    }
    const password = currentDiscoveryPassword()
    const request: DiscoverMailSettingsRequest = { emailAddress, password, route }
    const seq = ++discoverySeq.current
    setDiscovery({ kind: 'checking' })
    try {
      const result = await discoveryClient.discoverMailSettings(request)
      if (!mounted.current || seq !== discoverySeq.current) return
      const applied = appliedDiscovery(draftRef.current, result, password !== null, emailAddress)
      setDraft(applied.draft)
      setDiscovery(applied.discovery)
    } catch (cause) {
      if (!mounted.current || seq !== discoverySeq.current) return
      setDraft((value) => ({ ...value, ews: null }))
      setDiscovery({ kind: 'failed', status: 'network' })
      // A swallowed IPC or host failure would masquerade as a network problem;
      // surface whatever the host actually reported.
      showFailure(clientFailure(cause))
    }
  }

  function switchToManual() {
    discoverySeq.current += 1
    setDraft((value) => ({ ...value, ews: null }))
    setDiscovery({ kind: 'manual' })
  }

  /** Manual setup: the full form opens and its own lookup prefills it. */
  function enterManual() {
    if (signingIn.current) return
    setPhase('form')
  }

  /**
   * "Next" on the sign-in card: look the address up, apply the recommended
   * protocol (an Exchange endpoint when the server offers one, IMAP/SMTP
   * otherwise), verify the credentials against it, and submit on success. A
   * failed sign-in falls through to the manual form, prefilled with whatever
   * the lookup did find. Nothing is saved until the caller's `onSubmit` runs.
   */
  async function signIn() {
    if (!verificationClient || !discoveryClient || signingIn.current || pending.current) return
    setError(null)
    const current = draftRef.current
    const emailAddress = current.emailAddress.trim()
    if (!EMAIL_PATTERN.test(emailAddress)) {
      showFailure({ code: 'validation', field: 'emailAddress', message: '' })
      return
    }
    const password = currentDiscoveryPassword()
    if (!password) {
      showFailure({ code: 'validation', field: 'sharedPassword', message: '' })
      return
    }
    let route: RouteInput
    try {
      route = draftToRoute(current)
    } catch {
      // A half-entered proxy is validated on save; the lookup needs a route now.
      route = { kind: current.routeKind === 'inherit' ? 'inherit' : 'direct' }
    }
    signingIn.current = true
    setSignInState({ kind: 'checking' })
    onBusyChange(true)
    try {
      const result = await discoveryClient.discoverMailSettings({ emailAddress, password, route })
      if (!mounted.current) return
      const applied = appliedDiscovery(current, result, true, emailAddress)
      setDraft(applied.draft)
      setDiscovery(applied.discovery)
      if (result.status !== 'found' || (!result.ews && !result.imap && !result.smtp)) {
        // Nothing usable: the manual form explains what the lookup reported.
        setSignInState({ kind: 'failed' })
        setPhase('form')
        return
      }
      let connection: ConnectionInput
      try {
        connection = draftToConnection(applied.draft, account, provider)
      } catch (cause) {
        setSignInState({ kind: 'failed' })
        showFailure(clientFailure(cause))
        return
      }
      const report = await verificationClient.verifyDraftConnection(connection)
      if (!mounted.current) return
      setSignInState({ kind: 'report', report })
      if (report.imap.status !== 'verified' || report.smtp.status !== 'verified') {
        // Sign-in failed: hand the prefilled form over for manual correction.
        setLoginTest({ kind: 'ready', report, scopeKey: verificationResetKey })
        setPhase('form')
        return
      }
      pending.current = true
      setSaving(true)
      // Secrets exist only in this in-flight submission after it begins.
      setDraft(clearSecrets)
      try {
        await onSubmit(connection, applied.draft)
        onDirtyChange(false)
      } catch (cause) {
        showFailure(clientFailure(cause))
      } finally {
        pending.current = false
        setSaving(false)
      }
    } catch (cause) {
      if (!mounted.current) return
      setSignInState({ kind: 'failed' })
      showFailure(clientFailure(cause))
    } finally {
      signingIn.current = false
      if (mounted.current) onBusyChange(false)
    }
  }

  /** Manual Exchange details; the placeholder rows track the URL's host. */
  function changeEws(value: EwsConnectionConfig) {
    resetTest()
    setDraft((current) => {
      const host = ewsHost(value.url, current.emailAddress)
      return { ...current, ews: value, imap: { ...current.imap, host }, smtp: { ...current.smtp, host } }
    })
    onDirtyChange(true)
    setError(null)
  }

  /** Switching the receive protocol by hand, as an alternative to a lookup. */
  function switchProtocol(protocol: 'exchange' | 'imapSmtp') {
    resetTest()
    if (protocol === 'imapSmtp') {
      setDraft((current) => ({ ...current, ews: null }))
      setDiscovery({ kind: 'manual' })
      onDirtyChange(true)
      setError(null)
      return
    }
    setDraft((current) => {
      if (current.ews) return current
      const domain = current.emailAddress.trim().split('@')[1] ?? ''
      const host = domain ? `mail.${domain}` : ''
      const url = host ? `https://${host}/EWS/Exchange.asmx` : ''
      const login = current.emailAddress.trim()
      const placeholder: ConnectionDraft = { host, port: '443', tls: 'implicit', username: login }
      return {
        ...current,
        mode: 'shared',
        ews: { url, auth: 'ntlm', username: login, certPrincipalName: null },
        imap: { ...placeholder }, smtp: { ...placeholder },
      }
    })
    setDiscovery({ kind: 'manual' })
    onDirtyChange(true)
    setError(null)
  }

  async function testLogin() {
    if (!verificationClient || testPending.current || pending.current) return
    setError(null)
    setTestError(true)
    let connection: ConnectionInput
    try {
      connection = draftToConnection(draft, account, provider)
    } catch (cause) {
      resetTest()
      showFailure(clientFailure(cause))
      return
    }
    const controller = new AbortController()
    testPending.current = controller
    setLoginTest({ kind: 'checking', scopeKey: verificationResetKey })
    try {
      const report = await verificationClient.verifyDraftConnection(connection, controller.signal)
      if (testPending.current === controller && !controller.signal.aborted) {
        setLoginTest({ kind: 'ready', report, scopeKey: verificationResetKey })
      }
    } catch (cause) {
      if (testPending.current === controller && !controller.signal.aborted) {
        setLoginTest({ kind: 'idle', scopeKey: verificationResetKey })
        showFailure(clientFailure(cause))
      }
    } finally {
      if (testPending.current === controller) testPending.current = null
    }
  }

  async function save() {
    if (pending.current) return
    resetTest()
    setError(null)
    setTestError(false)
    let connection
    try {
      connection = draftToConnection(draft, account, provider)
    } catch (cause) {
      const failure = clientFailure(cause)
      setDraft(clearSecrets)
      showFailure(failure)
      return
    }
    const submitted = draft
    pending.current = true
    setSaving(true)
    onBusyChange(true)
    // Secrets exist only in this in-flight submission after it begins.
    setDraft(clearSecrets)
    try {
      await onSubmit(connection, submitted)
      onDirtyChange(false)
    } catch (cause) {
      showFailure(clientFailure(cause))
    } finally {
      pending.current = false
      setSaving(false)
      onBusyChange(false)
    }
  }

  const serverFields = (<><ConnectionFields protocol="imap" value={draft.imap} emailAddress={draft.emailAddress} onChange={(value) => change('imap', value)} error={error} />
          <ConnectionFields protocol="smtp" value={draft.smtp} emailAddress={draft.emailAddress} onChange={(value) => change('smtp', value)} error={error} /></>)
  const modeFields = oauth ? <FieldDescription>{t('accounts:oauthManagedCredentials')}</FieldDescription> : (<><RadioGroup value={draft.mode} onValueChange={(value) => changeMode(value as AuthenticationMode)} aria-label={t('accounts:authenticationMode')}>
                <Field orientation="horizontal"><RadioGroupItem id="auth-shared" value="shared" /><FieldLabel htmlFor="auth-shared">{t('accounts:sharedPassword')}</FieldLabel></Field>
                <Field orientation="horizontal"><RadioGroupItem id="auth-separate" value="separate" /><FieldLabel htmlFor="auth-separate">{t('accounts:separatePasswords')}</FieldLabel></Field>
              </RadioGroup>
              {account && !sameMode && <FieldDescription>{t('accounts:authenticationChanged')}</FieldDescription>}</>)
  const passwordFields = oauth ? null : (<>{draft.mode === 'shared'
                ? <PasswordField id="sharedPassword" label={credentialText.shared} value={draft.sharedPassword} onChange={(value) => change('sharedPassword', value)} canKeep={sameMode && !!password?.imap.passwordConfigured} error={error} placeholder={credentialText.placeholder} />
                : <>
                    <PasswordField id="imapPassword" label={credentialText.incoming} value={draft.imapPassword} onChange={(value) => change('imapPassword', value)} canKeep={sameMode && !!password?.imap.passwordConfigured} error={error} placeholder={credentialText.placeholder} />
                    <PasswordField id="smtpPassword" label={credentialText.outgoing} value={draft.smtpPassword} onChange={(value) => change('smtpPassword', value)} canKeep={sameMode && !!password?.smtp.passwordConfigured} error={error} placeholder={credentialText.placeholder} />
                  </>}</>)
  const routeFields = <RouteFields draft={draft} change={change} error={error} oldProxyAuth={oldProxyAuth} />
  const ewsMode = draft.ews !== null
  // Before an address is entered, the manual step shows no server fields at all.
  const discoveryIdle = discoveryEnabled && !account && discovery.kind === 'idle' && draft.emailAddress.trim() === ''
  const showServerFields = !ewsMode && !discoveryIdle
  const discoverySource = discovery.kind === 'found' || discovery.kind === 'ews' ? discovery.source : null

  if (oauth) return <SettingsPage><Alert role="status"><AlertTitle>{t('accounts:signedInThroughBrowser')}</AlertTitle><AlertDescription>{t('accounts:oauthConnectionUnavailable')}</AlertDescription></Alert></SettingsPage>

  // The sign-in card: address and password first. "Next" signs in with the
  // recommended protocol; "manual setup" opens the fully manual form.
  if (phase === 'signin') {
    const busy = signInState.kind === 'checking'
    return (
      <SettingsPage footer={<>
        <FieldDescription>{busy ? t('accounts:signInBusy') : t('accounts:credentialsNotDisplayed')}</FieldDescription>
        <div className="flex shrink-0 gap-2">
          {onBack && <Button variant="outline" onClick={() => { resetTest(); onBack() }} disabled={busy}>{t('accounts:back')}</Button>}
          <Button variant="outline" disabled={busy} onClick={enterManual}>{t('accounts:manualSetupAction')}</Button>
          <Button onClick={() => { void signIn() }} disabled={busy || unavailable || !verificationClient?.available}>{busy ? submittingLabel : submitLabel}</Button>
        </div>
      </>}>
        <Alert role="status">
          <AlertTitle>{t('accounts:signInHeading')}</AlertTitle>
          <AlertDescription>{t('accounts:signInHint')}</AlertDescription>
        </Alert>
        {error && (
          <Alert variant="destructive">
            <AlertTitle>{t('accounts:notSaved')}</AlertTitle>
            <AlertDescription>{errorText(error, errorContext)}</AlertDescription>
          </Alert>
        )}
        <FieldSet disabled={busy} aria-label={t('accounts:emailAccountSettings')} aria-busy={busy} className="gap-6">
          <SettingsGroup title={t('accounts:emailAddress')} description={t('accounts:signInEmailHint')}>
            <TextField id="emailAddress" label={t('accounts:emailAddress')} value={draft.emailAddress} onChange={(value) => change('emailAddress', value)} placeholder="you@example.com" inputMode="email" error={error} errorContext={errorContext} />
          </SettingsGroup>
          <SettingsGroup title={credentialText.shared}>
            <PasswordField id="sharedPassword" label={credentialText.shared} value={draft.sharedPassword} onChange={(value) => change('sharedPassword', value)} canKeep={false} error={error} placeholder={credentialText.placeholder} />
            <FieldDescription>{t('accounts:signInProtocolHint')}</FieldDescription>
          </SettingsGroup>
          <details className="text-sm">
            <summary className="cursor-pointer rounded-md py-2 font-medium focus-visible:outline-2 focus-visible:outline-ring">{t('accounts:advancedSettings')}</summary>
            <div className="flex flex-col gap-6 pt-4">{routeFields}</div>
          </details>
        </FieldSet>
        <div role="status" aria-live="polite" className="text-sm">
          {signInState.kind === 'checking' && <p className="text-muted-foreground">{t('accounts:signInBusy')}</p>}
          {signInState.kind === 'report' && verificationClient && <ConnectionVerificationReport report={signInState.report} client={verificationClient} providerId={provider.id} isEws={ewsMode} />}
          {signInState.kind === 'failed' && <p className="text-muted-foreground">{t('accounts:signInFailedHint')}</p>}
        </div>
      </SettingsPage>
    )
  }

  return (
    <SettingsPage footer={<>
      <FieldDescription>{saving ? t('accounts:savingSecurely') : t('accounts:credentialsNotDisplayed')}</FieldDescription>
      <div className="flex shrink-0 gap-2">
        {onBack && <Button variant="outline" onClick={() => { resetTest(); onBack() }} disabled={saving}>{t('accounts:back')}</Button>}
        <Button onClick={() => { void save() }} disabled={saving || unavailable}>{saving ? submittingLabel : submitLabel}</Button>
      </div>
    </>}>
        <Alert role="status">
          <AlertTitle>{intro.title}</AlertTitle>
          <AlertDescription>{intro.description}</AlertDescription>
          {setup && onOpenHelp && <Button variant="link" className="mt-1 h-auto w-fit px-0" disabled={saving} onClick={() => {
            setHelpFailed(false)
            void onOpenHelp().catch(() => setHelpFailed(true))
          }}>{credentialText.help}</Button>}
          {helpFailed && <FieldDescription role="alert">{t('accounts:helpOpenFailed')}</FieldDescription>}
        </Alert>
        {needsTrust && <Alert variant="destructive"><AlertTitle>{t('accounts:credentialsNeedConfirmation')}</AlertTitle><AlertDescription>{t('accounts:confirmChangedCredentials')}</AlertDescription></Alert>}
        {error && (
          <Alert variant="destructive">
            <AlertTitle>{testError ? t('accounts:loginTestFailed') : error.code === 'conflict' ? t('accounts:configurationChanged') : t('accounts:notSaved')}</AlertTitle>
            <AlertDescription>{errorText(error, errorContext)}  {!testError && t('accounts:passwordsCleared')}</AlertDescription>
            {onReload && (error.code === 'conflict' || error.code === 'transport' || error.code === 'idempotencyConflict') &&
              <Button variant="outline" className="mt-3 w-fit" onClick={onReload}>{t('accounts:reloadSettings')}</Button>}
          </Alert>
        )}
        <FieldSet disabled={saving} aria-label={t('accounts:emailAccountSettings')} aria-busy={saving} className="gap-6">
          <SettingsGroup title={t('accounts:emailAddress')} description={setup?.emailHint ?? t('accounts:emailAddressDescription')}>
            <TextField id="emailAddress" label={t('accounts:emailAddress')} value={draft.emailAddress} onChange={(value) => change('emailAddress', value)} placeholder={setup?.emailPlaceholder ?? "you@example.com"} inputMode="email" error={error} errorContext={errorContext} />
          </SettingsGroup>
          {setup && (
            <SettingsGroup title={credentialText.shared} description={setup.credentialHint}>
              {passwordFields}
              {setup.rangeHint && <FieldDescription>{setup.rangeHint}</FieldDescription>}
            </SettingsGroup>
          )}
          {setup ? (
            <details open={advanced} onToggle={(event) => setAdvanced(event.currentTarget.open)}>
              <summary className="cursor-pointer rounded-md py-2 text-sm font-medium focus-visible:outline-2 focus-visible:outline-ring">{t('accounts:advancedSettings')}</summary>
              <div className="flex flex-col gap-6 pt-4">
                {serverFields}
                <SettingsGroup title={t('accounts:authentication')} description={t('accounts:authenticationDescription')}>{modeFields}</SettingsGroup>
                {routeFields}
              </div>
            </details>
          ) : (
            <>
              <SettingsGroup title={t('accounts:protocolType')} description={t('accounts:protocolTypeHint')}>
                <RadioGroup value={ewsMode ? 'exchange' : 'imapSmtp'} onValueChange={(value) => switchProtocol(value as 'exchange' | 'imapSmtp')} aria-label={t('accounts:protocolType')}>
                  <Field orientation="horizontal"><RadioGroupItem id="protocol-exchange" value="exchange" /><FieldLabel htmlFor="protocol-exchange">{t('accounts:protocolExchange')}</FieldLabel></Field>
                  <Field orientation="horizontal"><RadioGroupItem id="protocol-imapSmtp" value="imapSmtp" /><FieldLabel htmlFor="protocol-imapSmtp">{t('accounts:protocolImapSmtp')}</FieldLabel></Field>
                  <Field orientation="horizontal"><RadioGroupItem id="protocol-pop3" value="pop3" disabled /><FieldLabel htmlFor="protocol-pop3" className="text-muted-foreground">{t('accounts:protocolPop3')}</FieldLabel></Field>
                </RadioGroup>
                <FieldDescription>{t('accounts:protocolPop3Hint')}</FieldDescription>
              </SettingsGroup>
              {discoveryEnabled && !ewsMode && (
                <DiscoveryPanel state={discovery} source={discoverySource} emailEntered={draft.emailAddress.trim() !== ''}
                  saving={saving} onRetry={() => { void discover() }} />
              )}
              {ewsMode && draft.ews && (
                <EwsFields ews={draft.ews} source={discoverySource} saving={saving} onManual={switchToManual} onChange={changeEws} />
              )}
              {showServerFields && serverFields}
              <SettingsGroup title={t('accounts:authentication')} description={t('accounts:authenticationDescription')}>
                {!ewsMode && modeFields}
                {passwordFields}
              </SettingsGroup>
              {routeFields}
            </>
          )}
        </FieldSet>
        {verificationClient && <SettingsGroup title={t('accounts:testLogin')} description={t('accounts:draftVerificationHint')}>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" disabled={saving || unavailable || !verificationClient.available || currentTest.kind === 'checking'} onClick={() => void testLogin()}>
              {t(currentTest.kind === 'checking' ? 'accounts:testingLogin' : 'accounts:testLogin')}
            </Button>
            {currentTest.kind === 'checking' && <Button variant="ghost" onClick={() => {
              testPending.current?.abort()
              testPending.current = null
              setLoginTest({ kind: 'cancelled', scopeKey: verificationResetKey })
            }}>{t('accounts:cancelLoginTest')}</Button>}
          </div>
          <div role="status" aria-live="polite" className="text-sm">
            {currentTest.kind === 'cancelled' && <p className="text-muted-foreground">{t('accounts:verificationCancelled')}</p>}
            {currentTest.kind === 'ready' && <ConnectionVerificationReport report={currentTest.report} client={verificationClient} providerId={provider.id} isEws={ewsMode} />}
          </div>
        </SettingsGroup>}
    </SettingsPage>
  )
}
