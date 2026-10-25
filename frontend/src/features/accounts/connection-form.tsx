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
import { changeDraft, clearSecrets, draftToConnection, initialDraft, type AccountDraft, type ConnectionDraft, type SecretDraft } from './account-draft'

type FormProps = {
  /** The saved account whose connection is edited; `null` for a new one. */
  account: Account | null
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
  inputMode?: 'numeric' | 'email' | 'text'
  error?: ClientFailure | null
  autoComplete?: string
}

function TextField({ id, label, value, onChange, placeholder, inputMode, error, autoComplete = 'off' }: InputFieldProps) {
  const invalid = error?.field === id
  return (
    <Field data-invalid={invalid}>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input id={id} value={value} onChange={(event) => onChange(event.target.value)} placeholder={placeholder}
        inputMode={inputMode} autoComplete={autoComplete} autoCapitalize="none" spellCheck={false}
        aria-invalid={invalid} aria-describedby={invalid ? `${id}-error` : undefined} />
      {invalid && <FieldError id={`${id}-error`}>{error.message}</FieldError>}
    </Field>
  )
}

function ConnectionFields({ protocol, value, onChange, error }: {
  protocol: 'imap' | 'smtp'
  value: ConnectionDraft
  onChange(value: ConnectionDraft): void
  error: ClientFailure | null
}) {
  const isImap = protocol === 'imap'
  const name = protocol.toUpperCase()
  return (
    <SettingsGroup title={isImap ? '收信服务器' : '发信服务器'} badge={<Badge variant="secondary">{name}</Badge>}
      description={isImap ? '填写服务商提供的 IMAP 参数。' : '收发服务器与登录用户名可以不同。'}>
        <TextField id={`${protocol}.host`} label={`${name} 主机`} value={value.host} error={error}
          placeholder={isImap ? 'imap.example.com' : 'smtp.example.com'} onChange={(host) => onChange({ ...value, host })} />
        <div className="grid grid-cols-2 gap-4">
          <TextField id={`${protocol}.port`} label={`${name} 端口`} value={value.port} inputMode="numeric" error={error}
            onChange={(port) => onChange({ ...value, port })} />
          <Field>
            <FieldLabel htmlFor={`${protocol}.tls`}>连接安全</FieldLabel>
            <Select value={value.tls} onValueChange={(tls: TlsMode | null) => { if (tls) onChange({ ...value, tls }) }}
              items={{ implicit: 'TLS', startTls: 'STARTTLS' }}>
              <SelectTrigger id={`${protocol}.tls`} className="w-full" aria-label={`${name} 连接安全`}><SelectValue /></SelectTrigger>
              <SelectContent><SelectItem value="implicit">TLS</SelectItem><SelectItem value="startTls">STARTTLS</SelectItem></SelectContent>
            </Select>
          </Field>
        </div>
        <TextField id={`${protocol}.username`} label={`${name} 登录用户名`} value={value.username}
          placeholder="通常为完整邮箱地址" error={error} onChange={(username) => onChange({ ...value, username })} />
    </SettingsGroup>
  )
}

function PasswordField({ id, label, value, onChange, canKeep, error }: {
  id: string
  label: string
  value: SecretDraft
  onChange(value: SecretDraft): void
  canKeep: boolean
  error: ClientFailure | null
}) {
  const invalid = error?.field === id
  return (
    <FieldGroup>
      {canKeep && (
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel htmlFor={`${id}-replace`}>替换{label}</FieldLabel>
            <FieldDescription>{value.replace ? '本次保存使用新输入的凭证。' : '继续使用已保存的凭证，密码不会回填。'}</FieldDescription>
          </FieldContent>
          <Switch id={`${id}-replace`} checked={value.replace} onCheckedChange={(replace) => onChange({ replace, value: '' })} />
        </Field>
      )}
      {value.replace && (
        <Field data-invalid={invalid}>
          <FieldLabel htmlFor={id}>{label}</FieldLabel>
          <Input id={id} type="password" value={value.value} onChange={(event) => onChange({ ...value, value: event.target.value })}
            autoComplete="new-password" autoCapitalize="none" spellCheck={false} placeholder={id === 'proxyPassword' ? '输入代理密码' : '输入密码或邮箱授权码'}
            aria-invalid={invalid} aria-describedby={invalid ? `${id}-error` : undefined} />
          {invalid && <FieldError id={`${id}-error`}>{error.message}</FieldError>}
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
  account, initialDraft: resume, initialError = null, intro, submitLabel, submittingLabel,
  onSubmit, onBusyChange, onDirtyChange, onReload, unavailable, onBack,
}: FormProps) {
  const [draft, setDraft] = useState(() => resume ?? initialDraft(account))
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

  async function save() {
    if (pending.current) return
    setError(null)
    let connection
    try {
      connection = draftToConnection(draft, account)
    } catch (cause) {
      const failure = clientFailure(cause)
      setDraft(clearSecrets)
      setError(failure)
      if (failure.field) document.getElementById(failure.field)?.focus()
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
      setError(clientFailure(cause))
    } finally {
      pending.current = false
      setSaving(false)
      onBusyChange(false)
    }
  }

  return (
    <SettingsPage footer={<>
      <FieldDescription>{saving ? '正在安全保存配置…' : '凭证不会显示在账号详情中。'}</FieldDescription>
      <div className="flex shrink-0 gap-2">
        {onBack && <Button variant="outline" onClick={onBack} disabled={saving}>上一步</Button>}
        <Button onClick={() => { void save() }} disabled={saving || unavailable}>{saving ? submittingLabel : submitLabel}</Button>
      </div>
    </>}>
        <Alert role="status">
          <AlertTitle>{intro.title}</AlertTitle>
          <AlertDescription>{intro.description}</AlertDescription>
        </Alert>
        {needsTrust && <Alert variant="destructive"><AlertTitle>凭证需要重新确认</AlertTitle><AlertDescription>服务器或登录身份已改变。请为受影响的收信、发信或代理配置重新输入密码，以确认新的使用范围。</AlertDescription></Alert>}
        {error && (
          <Alert variant="destructive">
            <AlertTitle>{error.code === 'conflict' ? '配置已在其他位置更新' : '尚未保存'}</AlertTitle>
            <AlertDescription>{error.message} 已输入的密码已清空，需要时请重新输入。</AlertDescription>
            {onReload && (error.code === 'conflict' || error.code === 'transport' || error.code === 'idempotencyConflict') &&
              <Button variant="outline" className="mt-3 w-fit" onClick={onReload}>重新加载配置</Button>}
          </Alert>
        )}
        <FieldSet disabled={saving} aria-label="邮箱账号配置" aria-busy={saving} className="gap-6">
          <SettingsGroup title="邮箱地址" description="收发邮件所用的地址。">
            <TextField id="emailAddress" label="邮箱地址" value={draft.emailAddress} onChange={(value) => change('emailAddress', value)} placeholder="you@example.com" inputMode="email" error={error} />
          </SettingsGroup>
          <ConnectionFields protocol="imap" value={draft.imap} onChange={(value) => change('imap', value)} error={error} />
          <ConnectionFields protocol="smtp" value={draft.smtp} onChange={(value) => change('smtp', value)} error={error} />
          <SettingsGroup title="认证凭证" description="支持密码、授权码与应用专用密码，安全保存在系统凭据库。">
              <RadioGroup value={draft.mode} onValueChange={(value) => changeMode(value as AuthenticationMode)} aria-label="收发认证方式">
                <Field orientation="horizontal"><RadioGroupItem id="auth-shared" value="shared" /><FieldLabel htmlFor="auth-shared">收发使用相同密码</FieldLabel></Field>
                <Field orientation="horizontal"><RadioGroupItem id="auth-separate" value="separate" /><FieldLabel htmlFor="auth-separate">收发分别设置密码</FieldLabel></Field>
              </RadioGroup>
              {account && !sameMode && <FieldDescription>认证方式已改变，请重新输入凭证以确认收发服务器的使用范围。</FieldDescription>}
              {draft.mode === 'shared'
                ? <PasswordField id="sharedPassword" label="收发密码" value={draft.sharedPassword} onChange={(value) => change('sharedPassword', value)} canKeep={sameMode && !!account?.authentication.imap.passwordConfigured} error={error} />
                : <>
                    <PasswordField id="imapPassword" label="收信密码" value={draft.imapPassword} onChange={(value) => change('imapPassword', value)} canKeep={sameMode && !!account?.authentication.imap.passwordConfigured} error={error} />
                    <PasswordField id="smtpPassword" label="发信密码" value={draft.smtpPassword} onChange={(value) => change('smtpPassword', value)} canKeep={sameMode && !!account?.authentication.smtp.passwordConfigured} error={error} />
                  </>}
          </SettingsGroup>
          <SettingsGroup title="网络路由" description="为这个账号指定连接方式。保存配置不会测试代理。">
              <Field>
                <FieldLabel htmlFor="routeKind">连接方式</FieldLabel>
                <Select value={draft.routeKind} onValueChange={(value) => {
                  if (!value) return
                  change('routeKind', value)
                  if (value !== 'custom') change('proxyPassword', { ...draft.proxyPassword, value: '' })
                }}
                  items={{ direct: '直接连接', inherit: '使用应用默认', custom: '自定义代理' }}>
                  <SelectTrigger id="routeKind" className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent><SelectItem value="direct">直接连接</SelectItem><SelectItem value="inherit">使用应用默认</SelectItem><SelectItem value="custom">自定义代理</SelectItem></SelectContent>
                </Select>
                <FieldDescription>{draft.routeKind === 'inherit' ? '应用默认目前为直接连接。' : draft.routeKind === 'direct' ? '该账号使用直接连接。' : '代理连接失败时不会回退直连；目标主机名由代理解析。'}</FieldDescription>
              </Field>
              {draft.routeKind === 'custom' && <>
                <div className="grid grid-cols-[1fr_1.5fr_0.75fr] gap-4">
                  <Field>
                    <FieldLabel htmlFor="proxyKind">代理类型</FieldLabel>
                    <Select value={draft.proxyKind} onValueChange={(value) => { if (value) change('proxyKind', value) }} items={{ socks5: 'SOCKS5', httpConnect: 'HTTP CONNECT' }}>
                      <SelectTrigger id="proxyKind" className="w-full"><SelectValue /></SelectTrigger>
                      <SelectContent><SelectItem value="socks5">SOCKS5</SelectItem><SelectItem value="httpConnect">HTTP CONNECT</SelectItem></SelectContent>
                    </Select>
                  </Field>
                  <TextField id="proxyHost" label="代理主机" value={draft.proxyHost} onChange={(value) => change('proxyHost', value)} placeholder="127.0.0.1" error={error} />
                  <TextField id="proxyPort" label="代理端口" value={draft.proxyPort} onChange={(value) => change('proxyPort', value)} inputMode="numeric" error={error} />
                </div>
                <Field orientation="horizontal">
                  <FieldContent><FieldLabel htmlFor="proxyAuth">代理需要认证</FieldLabel><FieldDescription>仅在代理服务要求时填写。</FieldDescription></FieldContent>
                  <Switch id="proxyAuth" checked={draft.proxyAuth} onCheckedChange={(value) => { change('proxyAuth', value); change('proxyPassword', { replace: !oldProxyAuth, value: '' }) }} />
                </Field>
                {draft.proxyAuth && <>
                  <TextField id="proxyUsername" label="代理用户名" value={draft.proxyUsername} onChange={(value) => change('proxyUsername', value)} error={error} />
                  <PasswordField id="proxyPassword" label="代理密码" value={draft.proxyPassword} onChange={(value) => change('proxyPassword', value)} canKeep={!!oldProxyAuth?.passwordConfigured} error={error} />
                </>}
                <Field orientation="horizontal">
                  <FieldContent>
                    <FieldLabel htmlFor="imageProxyDns">通过此代理加载 HTTPS 图片</FieldLabel>
                    <FieldDescription>由此代理解析并连接图片服务器，应用无法独立验证最终 IP。更改代理地址或身份后需重新开启。</FieldDescription>
                  </FieldContent>
                  <Switch id="imageProxyDns" checked={draft.imageProxyDns} onCheckedChange={(value) => change('imageProxyDns', value)} />
                </Field>
              </>}
          </SettingsGroup>
        </FieldSet>
    </SettingsPage>
  )
}
