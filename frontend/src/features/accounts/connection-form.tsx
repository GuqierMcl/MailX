import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useRef, useState, type ReactNode } from 'react'
import type { Account, AuthenticationMode, ConnectionInput, TlsMode } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { clientFailure, type ClientFailure } from '@/lib/mailx-client'
import { SettingsGroup, SettingsPage } from '@/features/settings/settings-layout'
import { providerOf, type MailProvider } from './providers'
import { changeDraft, clearSecrets, draftToConnection, initialDraft, type AccountDraft, type ConnectionDraft, type SecretDraft } from './account-draft'

type FormProps = {
  /** The saved account whose connection is edited; `null` for a new one. */
  account: Account | null
  provider?: MailProvider
  onOpenHelp?(): Promise<void>
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
  errorContext?: string
}

function TextField({ id, label, value, onChange, placeholder, description, inputMode, error, autoComplete = 'off', errorContext }: InputFieldProps) {
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

function PasswordField({ id, label, value, onChange, canKeep, error, authorizationCode = false }: {
  id: string
  label: string
  value: SecretDraft
  onChange(value: SecretDraft): void
  canKeep: boolean
  authorizationCode?: boolean
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
            autoComplete="new-password" autoCapitalize="none" spellCheck={false} placeholder={id === 'proxyPassword' ? t('accounts:enterProxyPassword') : authorizationCode ? t('accounts:enterAuthorizationCode') : t('accounts:enterPassword')}
            aria-invalid={invalid} aria-describedby={invalid ? `${id}-error` : undefined} />
          {invalid && <FieldError id={`${id}-error`}>{errorText(error)}</FieldError>}
        </Field>
      )}
    </FieldGroup>
  )
}

/**
 * How MailX reaches a mailbox: address, servers, credentials and route. Shared
 * by the add-account wizard and the "edit connection" dialog; the caller
 * decides what submitting means.
 */
export function ConnectionForm({
  account, provider: selectedProvider, onOpenHelp, initialDraft: resume, initialError = null, intro, submitLabel, submittingLabel,
  onSubmit, onBusyChange, onDirtyChange, onReload, unavailable, onBack,
}: FormProps) {
  useTranslation()
  const provider = selectedProvider ?? providerOf(account?.providerId ?? null)
  const isQq = provider.id === 'qq'
  const [draft, setDraft] = useState(() => resume ?? initialDraft(account, provider))
  const [advanced, setAdvanced] = useState(!!account || !!resume)
  const [helpFailed, setHelpFailed] = useState(false)
  const [error, setError] = useState<ClientFailure | null>(initialError)
  const [saving, setSaving] = useState(false)
  const pending = useRef(false)
  const sameMode = account?.authentication.mode === draft.mode
  const oldProxyAuth = account?.route.kind === 'custom' ? account.route.authentication : null
  const needsTrust = account && (account.authentication.imap.trust === 'needsConfirmation'
    || account.authentication.smtp.trust === 'needsConfirmation'
    || oldProxyAuth?.trust === 'needsConfirmation')

  function change<K extends keyof AccountDraft>(key: K, value: AccountDraft[K]) {
    setDraft((current) => changeDraft(current, key, value))
    onDirtyChange(true)
    setError(null)
  }

  function changeMode(mode: AuthenticationMode) {
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

  async function save() {
    if (pending.current) return
    setError(null)
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
  const modeFields = (<><RadioGroup value={draft.mode} onValueChange={(value) => changeMode(value as AuthenticationMode)} aria-label={t('accounts:authenticationMode')}>
                <Field orientation="horizontal"><RadioGroupItem id="auth-shared" value="shared" /><FieldLabel htmlFor="auth-shared">{t('accounts:sharedPassword')}</FieldLabel></Field>
                <Field orientation="horizontal"><RadioGroupItem id="auth-separate" value="separate" /><FieldLabel htmlFor="auth-separate">{t('accounts:separatePasswords')}</FieldLabel></Field>
              </RadioGroup>
              {account && !sameMode && <FieldDescription>{t('accounts:authenticationChanged')}</FieldDescription>}</>)
  const passwordFields = (<>{draft.mode === 'shared'
                ? <PasswordField id="sharedPassword" label={t(isQq ? 'accounts:authorizationCode' : 'accounts:sharedPasswordLabel')} value={draft.sharedPassword} onChange={(value) => change('sharedPassword', value)} canKeep={sameMode && !!account?.authentication.imap.passwordConfigured} error={error} authorizationCode={isQq} />
                : <>
                    <PasswordField id="imapPassword" label={t(isQq ? 'accounts:incomingAuthorizationCode' : 'accounts:incomingPassword')} value={draft.imapPassword} onChange={(value) => change('imapPassword', value)} canKeep={sameMode && !!account?.authentication.imap.passwordConfigured} error={error} authorizationCode={isQq} />
                    <PasswordField id="smtpPassword" label={t(isQq ? 'accounts:outgoingAuthorizationCode' : 'accounts:outgoingPassword')} value={draft.smtpPassword} onChange={(value) => change('smtpPassword', value)} canKeep={sameMode && !!account?.authentication.smtp.passwordConfigured} error={error} authorizationCode={isQq} />
                  </>}</>)
  const routeFields = (<SettingsGroup title={t('accounts:networkRoute')} description={t('accounts:routeDescription')}>
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
                  <PasswordField id="proxyPassword" label={t('accounts:proxyPassword')} value={draft.proxyPassword} onChange={(value) => change('proxyPassword', value)} canKeep={!!oldProxyAuth?.passwordConfigured} error={error} />
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

  return (
    <SettingsPage footer={<>
      <FieldDescription>{saving ? t('accounts:savingSecurely') : t('accounts:credentialsNotDisplayed')}</FieldDescription>
      <div className="flex shrink-0 gap-2">
        {onBack && <Button variant="outline" onClick={onBack} disabled={saving}>{t('accounts:back')}</Button>}
        <Button onClick={() => { void save() }} disabled={saving || unavailable}>{saving ? submittingLabel : submitLabel}</Button>
      </div>
    </>}>
        <Alert role="status">
          <AlertTitle>{intro.title}</AlertTitle>
          <AlertDescription>{intro.description}</AlertDescription>
          {isQq && onOpenHelp && <Button variant="link" className="mt-1 h-auto w-fit px-0" disabled={saving} onClick={() => {
            setHelpFailed(false)
            void onOpenHelp().catch(() => setHelpFailed(true))
          }}>{t('accounts:getAuthorizationCode')}</Button>}
          {helpFailed && <FieldDescription role="alert">{t('accounts:helpOpenFailed')}</FieldDescription>}
        </Alert>
        {needsTrust && <Alert variant="destructive"><AlertTitle>{t('accounts:credentialsNeedConfirmation')}</AlertTitle><AlertDescription>{t('accounts:confirmChangedCredentials')}</AlertDescription></Alert>}
        {error && (
          <Alert variant="destructive">
            <AlertTitle>{error.code === 'conflict' ? t('accounts:configurationChanged') : t('accounts:notSaved')}</AlertTitle>
            <AlertDescription>{errorText(error, isQq ? 'qq' : undefined)}  {t('accounts:passwordsCleared')}</AlertDescription>
            {onReload && (error.code === 'conflict' || error.code === 'transport' || error.code === 'idempotencyConflict') &&
              <Button variant="outline" className="mt-3 w-fit" onClick={onReload}>{t('accounts:reloadSettings')}</Button>}
          </Alert>
        )}
        <FieldSet disabled={saving} aria-label={t('accounts:emailAccountSettings')} aria-busy={saving} className="gap-6">
          <SettingsGroup title={t('accounts:emailAddress')} description={t(isQq ? 'accounts:qqEmailHint' : 'accounts:emailAddressDescription')}>
            <TextField id="emailAddress" label={t('accounts:emailAddress')} value={draft.emailAddress} onChange={(value) => change('emailAddress', value)} placeholder={isQq ? "you@qq.com / you@foxmail.com" : "you@example.com"} inputMode="email" error={error} errorContext={isQq ? "qq" : undefined} />
          </SettingsGroup>
          {isQq && (
            <SettingsGroup title={t('accounts:authorizationCode')} description={t('accounts:qqAuthorizationCodeHint')}>
              {passwordFields}
              <FieldDescription>{t('accounts:qqRangeHint')}</FieldDescription>
            </SettingsGroup>
          )}
          {isQq ? (
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
              {serverFields}
              <SettingsGroup title={t('accounts:authentication')} description={t('accounts:authenticationDescription')}>
                {modeFields}
                {passwordFields}
              </SettingsGroup>
              {routeFields}
            </>
          )}
        </FieldSet>
    </SettingsPage>
  )
}
