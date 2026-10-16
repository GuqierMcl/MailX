import type { Account, AccountConfigInput, AuthenticationMode, ConnectionConfig, ProxyKind, SecretUpdate } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'

export type ConnectionDraft = Omit<ConnectionConfig, 'port'> & { port: string }
export type SecretDraft = { replace: boolean; value: string }

// Transient form state, intentionally distinct from the Rust-generated wire types.
export type AccountDraft = {
  displayName: string
  emailAddress: string
  imap: ConnectionDraft
  smtp: ConnectionDraft
  mode: AuthenticationMode
  sharedPassword: SecretDraft
  imapPassword: SecretDraft
  smtpPassword: SecretDraft
  routeKind: AccountConfigInput['route']['kind']
  proxyKind: ProxyKind
  proxyHost: string
  proxyPort: string
  proxyAuth: boolean
  proxyUsername: string
  proxyPassword: SecretDraft
}

export function initialDraft(account: Account | null): AccountDraft {
  const proxy = account?.route.kind === 'custom' ? account.route : null
  const secret = (configured?: boolean): SecretDraft => ({ replace: !configured, value: '' })
  return {
    displayName: account?.displayName ?? '',
    emailAddress: account?.emailAddress ?? '',
    imap: account ? { ...account.imap, port: String(account.imap.port) } : { host: '', port: '993', tls: 'implicit', username: '' },
    smtp: account ? { ...account.smtp, port: String(account.smtp.port) } : { host: '', port: '465', tls: 'implicit', username: '' },
    mode: account?.authentication.mode ?? 'shared',
    sharedPassword: secret(account?.authentication.imap.passwordConfigured),
    imapPassword: secret(account?.authentication.imap.passwordConfigured),
    smtpPassword: secret(account?.authentication.smtp.passwordConfigured),
    routeKind: account?.route.kind ?? 'direct',
    proxyKind: proxy?.proxyKind ?? 'socks5',
    proxyHost: proxy?.host ?? '',
    proxyPort: String(proxy?.port ?? 1080),
    proxyAuth: proxy?.authentication != null,
    proxyUsername: proxy?.authentication?.username ?? '',
    proxyPassword: secret(proxy?.authentication?.passwordConfigured),
  }
}

export function clearSecrets(draft: AccountDraft): AccountDraft {
  return {
    ...draft,
    sharedPassword: { ...draft.sharedPassword, value: '' },
    imapPassword: { ...draft.imapPassword, value: '' },
    smtpPassword: { ...draft.smtpPassword, value: '' },
    proxyPassword: { ...draft.proxyPassword, value: '' },
  }
}

function invalid(field: string, message: string): never {
  throw new MailxClientError({ code: 'validation', field, message })
}

function port(value: string, field: string): number {
  if (!/^\d+$/.test(value) || Number(value) < 1 || Number(value) > 65535) {
    invalid(field, '端口需要是 1 到 65535 之间的整数。')
  }
  return Number(value)
}

function host(value: string, field: string): string {
  const result = value.trim()
  if (!result || /[\s/@?#]/.test(result) || result.includes('://')) {
    invalid(field, '请填写主机名或 IP 地址，不要包含协议、路径、用户名或密码。')
  }
  return result
}

function connection(value: ConnectionDraft, protocol: 'imap' | 'smtp'): ConnectionConfig {
  if (!value.username.trim()) invalid(`${protocol}.username`, '请填写协议登录用户名。')
  return { ...value, host: host(value.host, `${protocol}.host`), port: port(value.port, `${protocol}.port`), username: value.username.trim() }
}

function secret(value: SecretDraft, field: string): SecretUpdate {
  if (!value.replace) return { action: 'keep' }
  if (!value.value) invalid(field, '请重新输入密码或授权码，空白不会覆盖已保存的凭证。')
  return { action: 'replace', value: value.value }
}

export function draftToConfig(draft: AccountDraft, account: Account | null): AccountConfigInput {
  if (!draft.displayName.trim()) invalid('displayName', '请填写账号名称。')
  if (!/^[^\s@]+@[^\s@]+$/.test(draft.emailAddress.trim())) invalid('emailAddress', '请填写有效的邮箱地址。')
  const imap = connection(draft.imap, 'imap')
  const smtp = connection(draft.smtp, 'smtp')
  const authentication: AccountConfigInput['authentication'] = draft.mode === 'shared'
    ? { kind: 'password', mode: 'shared', password: secret(draft.sharedPassword, 'sharedPassword') }
    : { kind: 'password', mode: 'separate', imapPassword: secret(draft.imapPassword, 'imapPassword'), smtpPassword: secret(draft.smtpPassword, 'smtpPassword') }
  let route: AccountConfigInput['route'] = { kind: draft.routeKind === 'inherit' ? 'inherit' : 'direct' }
  if (draft.routeKind === 'custom') {
    if (draft.proxyAuth && !draft.proxyUsername.trim()) invalid('proxyUsername', '请填写代理用户名。')
    route = {
      kind: 'custom', proxyKind: draft.proxyKind,
      host: host(draft.proxyHost, 'proxyHost'), port: port(draft.proxyPort, 'proxyPort'), dns: 'proxy',
      authentication: draft.proxyAuth ? { username: draft.proxyUsername.trim(), password: secret(draft.proxyPassword, 'proxyPassword') } : null,
    }
  }
  return { displayName: draft.displayName.trim(), emailAddress: draft.emailAddress.trim(), providerId: account?.providerId ?? null, imap, smtp, authentication, route }
}
