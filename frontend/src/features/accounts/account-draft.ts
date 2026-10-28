import { t } from '@/i18n'
import type { Account, AuthenticationMode, ConnectionConfig, ConnectionInput, ProxyKind, SecretUpdate } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import type { MailProvider } from './providers'

export type ConnectionDraft = Omit<ConnectionConfig, 'port'> & { port: string }
export type SecretDraft = { replace: boolean; value: string }

// Transient connection form state, intentionally distinct from the Rust-generated wire types.
export type AccountDraft = {
  emailAddress: string
  imap: ConnectionDraft
  smtp: ConnectionDraft
  mode: AuthenticationMode
  sharedPassword: SecretDraft
  imapPassword: SecretDraft
  smtpPassword: SecretDraft
  routeKind: ConnectionInput['route']['kind']
  proxyKind: ProxyKind
  proxyHost: string
  proxyPort: string
  proxyAuth: boolean
  proxyUsername: string
  proxyPassword: SecretDraft
  imageProxyDns: boolean
  confirmImageProxyDns: boolean
}

export function initialDraft(account: Account | null, provider?: MailProvider): AccountDraft {
  const proxy = account?.route.kind === 'custom' ? account.route : null
  const secret = (configured?: boolean): SecretDraft => ({ replace: !configured, value: '' })
  return {
    emailAddress: account?.emailAddress ?? '',
    imap: account ? { ...account.imap, port: String(account.imap.port) } : { host: '', tls: 'implicit', ...provider?.preset?.imap, port: String(provider?.preset?.imap.port ?? 993), username: '' },
    smtp: account ? { ...account.smtp, port: String(account.smtp.port) } : { host: '', tls: 'implicit', ...provider?.preset?.smtp, port: String(provider?.preset?.smtp.port ?? 465), username: '' },
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
    imageProxyDns: proxy?.imageProxyDns ?? false,
    confirmImageProxyDns: false,
  }
}

/** An image permission follows only the proxy identity the user selected. */
export function changeDraft<K extends keyof AccountDraft>(draft: AccountDraft, key: K, value: AccountDraft[K]): AccountDraft {
  const changed = { ...draft, [key]: value }
  if (key === 'imageProxyDns') {
    changed.confirmImageProxyDns = value === true
  } else if (['routeKind', 'proxyKind', 'proxyHost', 'proxyPort', 'proxyAuth', 'proxyUsername'].includes(key) && value !== draft[key]) {
    changed.imageProxyDns = false
    changed.confirmImageProxyDns = false
  }
  return changed
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
    invalid(field, t('accounts:invalidPort'))
  }
  return Number(value)
}

function host(value: string, field: string): string {
  const result = value.trim()
  if (!result || /[\s/@?#]/.test(result) || result.includes('://')) {
    invalid(field, t('accounts:invalidHost'))
  }
  return result
}

function connection(value: ConnectionDraft, protocol: 'imap' | 'smtp', emailAddress: string): ConnectionConfig {
  // Use Unicode White_Space plus U+FEFF at both the JS and Rust input boundaries.
  const username = value.username.replace(/^[\s\u0085]+|[\s\u0085]+$/g, '') || emailAddress
  return { ...value, host: host(value.host, `${protocol}.host`), port: port(value.port, `${protocol}.port`), username }
}

function secret(value: SecretDraft, field: string): SecretUpdate {
  if (!value.replace) return { action: 'keep' }
  if (!value.value) invalid(field, t('accounts:passwordRequired'))
  return { action: 'replace', value: value.value }
}

/** Validates the draft into a connection; throws a field error for the first problem. */
export function draftToConnection(draft: AccountDraft, account: Account | null, provider?: MailProvider): ConnectionInput {
  const emailAddress = draft.emailAddress.trim()
  if (!/^[^\s@]+@[^\s@]+$/.test(emailAddress)) invalid('emailAddress', t('accounts:invalidEmail'))
  if (!account && provider?.preset && !provider.preset.domains.includes(emailAddress.split('@')[1].toLowerCase())) {
    invalid('emailAddress', t('accounts:qqEmailRequired'))
  }
  const imap = connection(draft.imap, 'imap', emailAddress)
  const smtp = connection(draft.smtp, 'smtp', emailAddress)
  const authentication: ConnectionInput['authentication'] = draft.mode === 'shared'
    ? { kind: 'password', mode: 'shared', password: secret(draft.sharedPassword, 'sharedPassword') }
    : { kind: 'password', mode: 'separate', imapPassword: secret(draft.imapPassword, 'imapPassword'), smtpPassword: secret(draft.smtpPassword, 'smtpPassword') }
  let route: ConnectionInput['route'] = { kind: draft.routeKind === 'inherit' ? 'inherit' : 'direct' }
  if (draft.routeKind === 'custom') {
    if (draft.proxyAuth && !draft.proxyUsername.trim()) invalid('proxyUsername', t('accounts:proxyUsernameRequired'))
    route = {
      kind: 'custom', proxyKind: draft.proxyKind,
      host: host(draft.proxyHost, 'proxyHost'), port: port(draft.proxyPort, 'proxyPort'), dns: 'proxy',
      imageProxyDns: draft.imageProxyDns, confirmImageProxyDns: draft.confirmImageProxyDns,
      authentication: draft.proxyAuth ? { username: draft.proxyUsername.trim(), password: secret(draft.proxyPassword, 'proxyPassword') } : null,
    }
  }
  return { emailAddress, providerId: account ? account.providerId : provider?.id ?? null, imap, smtp, authentication, route }
}
