import { useRef, useState } from 'react'
import type { Account, AuthenticationMode, TlsMode } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { clearSecrets, draftToConfig, initialDraft, type AccountDraft, type ConnectionDraft, type SecretDraft } from './account-draft'

type FormProps = {
  account: Account | null
  client: MailxClient
  onSaved(account: Account): void
  onBusyChange(busy: boolean): void
  onDirtyChange(dirty: boolean): void
  onReload(): void
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
  return (
    <Card>
      <CardHeader>
        <CardTitle role="heading" aria-level={2}>{isImap ? '收信服务器' : '发信服务器'} <Badge variant="secondary">{protocol.toUpperCase()}</Badge></CardTitle>
        <CardDescription>{isImap ? '填写服务商提供的 IMAP 参数。' : '收发服务器与登录用户名可以不同。'}</CardDescription>
      </CardHeader>
      <CardContent>
        <FieldGroup>
          <TextField id={`${protocol}.host`} label={`${protocol.toUpperCase()} 主机`} value={value.host} error={error}
            placeholder={isImap ? 'imap.example.com' : 'smtp.example.com'} onChange={(host) => onChange({ ...value, host })} />
          <FieldGroup className="connection-options">
            <TextField id={`${protocol}.port`} label={`${protocol.toUpperCase()} 端口`} value={value.port} inputMode="numeric" error={error}
              onChange={(port) => onChange({ ...value, port })} />
            <Field>
              <FieldLabel htmlFor={`${protocol}.tls`}>连接安全</FieldLabel>
              <Select value={value.tls} onValueChange={(tls: TlsMode | null) => { if (tls) onChange({ ...value, tls }) }}
                items={{ implicit: 'TLS', startTls: 'STARTTLS' }}>
                <SelectTrigger id={`${protocol}.tls`} className="w-full" aria-label={`${protocol.toUpperCase()} 连接安全`}><SelectValue /></SelectTrigger>
                <SelectContent><SelectItem value="implicit">TLS</SelectItem><SelectItem value="startTls">STARTTLS</SelectItem></SelectContent>
              </Select>
            </Field>
          </FieldGroup>
          <TextField id={`${protocol}.username`} label={`${protocol.toUpperCase()} 登录用户名`} value={value.username}
            placeholder="通常为完整邮箱地址" error={error} onChange={(username) => onChange({ ...value, username })} />
        </FieldGroup>
      </CardContent>
    </Card>
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

export function AccountForm({ account, client, onSaved, onBusyChange, onDirtyChange, onReload }: FormProps) {
  const [draft, setDraft] = useState(() => initialDraft(account))
  const [error, setError] = useState<ClientFailure | null>(null)
  const [saving, setSaving] = useState(false)
  const pending = useRef(false)
  const requestId = useRef(crypto.randomUUID())
  const sameMode = account?.authentication.mode === draft.mode
  const oldProxyAuth = account?.route.kind === 'custom' ? account.route.authentication : null
  const needsTrust = account && (account.authentication.imap.trust === 'needsConfirmation'
    || account.authentication.smtp.trust === 'needsConfirmation'
    || oldProxyAuth?.trust === 'needsConfirmation')

  function change<K extends keyof AccountDraft>(key: K, value: AccountDraft[K]) {
    setDraft((current) => ({ ...current, [key]: value }))
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
    let config
    try {
      config = draftToConfig(draft, account)
    } catch (cause) {
      const failure = clientFailure(cause)
      setDraft(clearSecrets)
      setError(failure)
      if (failure.field) document.getElementById(failure.field)?.focus()
      return
    }
    pending.current = true
    setSaving(true)
    onBusyChange(true)
    // Secrets exist only in this in-flight request after a submission begins.
    setDraft(clearSecrets)
    try {
      const saved = account
        ? await client.updateAccount({ accountId: account.id, expectedRevision: account.revision, config })
        : await client.createAccount({ requestId: requestId.current, config })
      onDirtyChange(false)
      onSaved(saved)
    } catch (cause) {
      setError(clientFailure(cause))
    } finally {
      pending.current = false
      setSaving(false)
      onBusyChange(false)
    }
  }

  return (
    <FieldGroup className="account-form">
      <Card className="detail-heading">
        <CardHeader>
          <CardTitle className="text-xl" role="heading" aria-level={1}>{account ? account.displayName : '添加邮箱账号'}</CardTitle>
          <CardDescription>{account ? account.emailAddress : '把邮箱配置保存在这台设备上。'}</CardDescription>
        </CardHeader>
      </Card>
      <Alert className="status-note" role="status">
        <AlertTitle>{account ? '尚未验证连接' : '手动配置 · 密码或授权码'}</AlertTitle>
        <AlertDescription>此处只保存账号设置，不会连接服务器或收发邮件。</AlertDescription>
      </Alert>
      {needsTrust && <Alert variant="destructive"><AlertTitle>凭证需要重新确认</AlertTitle><AlertDescription>服务器或登录身份已改变。请为受影响的收信、发信或代理配置重新输入密码，以确认新的使用范围。</AlertDescription></Alert>}
      {error && (
        <Alert variant="destructive" className="save-error">
          <AlertTitle>{error.code === 'conflict' ? '配置已在其他位置更新' : '尚未保存'}</AlertTitle>
          <AlertDescription>{error.message} 已输入的密码已清空，需要时请重新输入。</AlertDescription>
          {(error.code === 'conflict' || error.code === 'transport' || error.code === 'idempotencyConflict') &&
            <Button variant="outline" className="mt-3 w-fit" onClick={onReload}>重新加载配置</Button>}
        </Alert>
      )}
      <FieldSet disabled={saving} aria-label="邮箱账号配置" aria-busy={saving}>
        <Card>
          <CardHeader><CardTitle role="heading" aria-level={2}>基本信息</CardTitle><CardDescription>账号名称只用于 MailX 中的显示。</CardDescription></CardHeader>
          <CardContent>
            <FieldGroup className="basic-fields">
              <TextField id="displayName" label="账号名称" value={draft.displayName} onChange={(value) => change('displayName', value)} placeholder="例如：工作邮箱" error={error} />
              <TextField id="emailAddress" label="邮箱地址" value={draft.emailAddress} onChange={(value) => change('emailAddress', value)} placeholder="you@example.com" inputMode="email" error={error} />
            </FieldGroup>
          </CardContent>
        </Card>
        <FieldGroup className="connection-grid">
          <ConnectionFields protocol="imap" value={draft.imap} onChange={(value) => change('imap', value)} error={error} />
          <ConnectionFields protocol="smtp" value={draft.smtp} onChange={(value) => change('smtp', value)} error={error} />
        </FieldGroup>
        <Card>
          <CardHeader><CardTitle role="heading" aria-level={2}>认证凭证</CardTitle><CardDescription>支持密码、授权码与应用专用密码，安全保存在系统凭据库。</CardDescription></CardHeader>
          <CardContent>
            <FieldGroup>
              <FieldSet>
                <FieldLegend>收发认证方式</FieldLegend>
                <RadioGroup value={draft.mode} onValueChange={(value) => changeMode(value as AuthenticationMode)} className="auth-choice" aria-label="收发认证方式">
                  <Field orientation="horizontal"><RadioGroupItem id="auth-shared" value="shared" /><FieldLabel htmlFor="auth-shared">收发使用相同密码</FieldLabel></Field>
                  <Field orientation="horizontal"><RadioGroupItem id="auth-separate" value="separate" /><FieldLabel htmlFor="auth-separate">收发分别设置密码</FieldLabel></Field>
                </RadioGroup>
              </FieldSet>
              {account && !sameMode && <FieldDescription>认证方式已改变，请重新输入凭证以确认收发服务器的使用范围。</FieldDescription>}
              {draft.mode === 'shared'
                ? <PasswordField id="sharedPassword" label="收发密码" value={draft.sharedPassword} onChange={(value) => change('sharedPassword', value)} canKeep={sameMode && !!account?.authentication.imap.passwordConfigured} error={error} />
                : <FieldGroup className="separate-passwords">
                    <PasswordField id="imapPassword" label="收信密码" value={draft.imapPassword} onChange={(value) => change('imapPassword', value)} canKeep={sameMode && !!account?.authentication.imap.passwordConfigured} error={error} />
                    <PasswordField id="smtpPassword" label="发信密码" value={draft.smtpPassword} onChange={(value) => change('smtpPassword', value)} canKeep={sameMode && !!account?.authentication.smtp.passwordConfigured} error={error} />
                  </FieldGroup>}
            </FieldGroup>
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle role="heading" aria-level={2}>账号网络路由</CardTitle><CardDescription>为这个账号指定连接方式。保存配置不会测试代理。</CardDescription></CardHeader>
          <CardContent>
            <FieldGroup>
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
                <FieldGroup className="proxy-fields">
                  <Field>
                    <FieldLabel htmlFor="proxyKind">代理类型</FieldLabel>
                    <Select value={draft.proxyKind} onValueChange={(value) => { if (value) change('proxyKind', value) }} items={{ socks5: 'SOCKS5', httpConnect: 'HTTP CONNECT' }}>
                      <SelectTrigger id="proxyKind" className="w-full"><SelectValue /></SelectTrigger>
                      <SelectContent><SelectItem value="socks5">SOCKS5</SelectItem><SelectItem value="httpConnect">HTTP CONNECT</SelectItem></SelectContent>
                    </Select>
                  </Field>
                  <TextField id="proxyHost" label="代理主机" value={draft.proxyHost} onChange={(value) => change('proxyHost', value)} placeholder="127.0.0.1" error={error} />
                  <TextField id="proxyPort" label="代理端口" value={draft.proxyPort} onChange={(value) => change('proxyPort', value)} inputMode="numeric" error={error} />
                </FieldGroup>
                <Field orientation="horizontal">
                  <FieldContent><FieldLabel htmlFor="proxyAuth">代理需要认证</FieldLabel><FieldDescription>仅在代理服务要求时填写。</FieldDescription></FieldContent>
                  <Switch id="proxyAuth" checked={draft.proxyAuth} onCheckedChange={(value) => { change('proxyAuth', value); change('proxyPassword', { replace: !oldProxyAuth, value: '' }) }} />
                </Field>
                {draft.proxyAuth && <FieldGroup className="separate-passwords">
                  <TextField id="proxyUsername" label="代理用户名" value={draft.proxyUsername} onChange={(value) => change('proxyUsername', value)} error={error} />
                  <PasswordField id="proxyPassword" label="代理密码" value={draft.proxyPassword} onChange={(value) => change('proxyPassword', value)} canKeep={!!oldProxyAuth?.passwordConfigured} error={error} />
                </FieldGroup>}
              </>}
            </FieldGroup>
          </CardContent>
        </Card>
      </FieldSet>
      <Card className="save-bar">
        <CardContent className="save-bar-content">
          <FieldDescription>{saving ? '正在安全保存配置…' : '凭证不会显示在账号详情中。'}</FieldDescription>
          <Button size="lg" onClick={() => { void save() }} disabled={saving || !client.available}>{saving ? '保存中…' : account ? '保存更改' : '保存账号'}</Button>
        </CardContent>
      </Card>
    </FieldGroup>
  )
}
