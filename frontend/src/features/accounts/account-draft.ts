import { t } from '@/i18n'
import type { Account, AuthenticationMode, ConnectionConfig, ConnectionInput, ProxyKind, RouteInput, SecretUpdate } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { providerEmailRequired, type MailProvider } from './providers'

export type ConnectionDraft = Omit<ConnectionConfig, 'port'> & { port: string }
export type SecretDraft = { replace: boolean; value: string }
type Protocol = 'imap' | 'smtp'
type EndpointField = 'host' | 'port' | 'tls'

// Transient connection form state, intentionally distinct from the Rust-generated wire types.
export type AccountDraft = {
  emailAddress: string
  /** Only used by the separate Exchange form; never interpreted by IMAP/SMTP. */
  ews?: { url: string; username: string; manual: boolean }
  imap: ConnectionDraft
  smtp: ConnectionDraft
  /** Track explicit edits only for new presets with domain-specific defaults. */
  endpointOverrides?: Partial<Record<Protocol, Partial<Record<EndpointField, true>>>>
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
  const password = account?.authentication.kind === 'password' ? account.authentication : null
  const proxy = account?.route.kind === 'custom' ? account.route : null
  const secret = (configured?: boolean): SecretDraft => ({ replace: !configured, value: '' })
  return {
    emailAddress: account?.emailAddress ?? '',
    imap: account && 'imap' in account ? { ...account.imap, port: String(account.imap.port) } : { host: '', tls: 'implicit', ...provider?.preset?.imap, port: String(provider?.preset?.imap.port ?? 993), username: '' },
    smtp: account && 'smtp' in account ? { ...account.smtp, port: String(account.smtp.port) } : { host: '', tls: 'implicit', ...provider?.preset?.smtp, port: String(provider?.preset?.smtp.port ?? 465), username: '' },
    ...(!account && provider?.preset?.serversByDomain ? { endpointOverrides: {} } : {}),
    mode: password?.mode ?? 'shared',
    sharedPassword: secret(password?.imap.passwordConfigured),
    imapPassword: secret(password?.imap.passwordConfigured),
    smtpPassword: secret(password?.smtp.passwordConfigured),
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

export function changeDraftServer<K extends keyof ConnectionDraft>(draft: AccountDraft, protocol: Protocol, field: K, value: ConnectionDraft[K]): AccountDraft {
  const next = { ...draft, [protocol]: { ...draft[protocol], [field]: value } }
  if (draft.endpointOverrides && field !== 'username') {
    next.endpointOverrides = { ...draft.endpointOverrides, [protocol]: { ...draft.endpointOverrides[protocol], [field]: true } }
  }
  return next
}

function emailDomain(emailAddress: string): string | undefined {
  return /^[^\s@]+@([^\s@]+)$/.exec(emailAddress.trim())?.[1].toLowerCase()
}

/** Domain-based defaults never change saved connections or explicit field edits. */
export function draftServer(draft: AccountDraft, protocol: Protocol, newProvider?: MailProvider): ConnectionDraft {
  const value = draft[protocol]
  const preset = newProvider?.preset
  if (!preset) return value
  if (preset.domains === 'organization') {
    return { ...value, ...preset[protocol], port: String(preset[protocol].port) }
  }
  if (!preset.serversByDomain) return value
  const domain = emailDomain(draft.emailAddress)
  const defaults = (domain && preset.domains.includes(domain) ? preset.serversByDomain[domain] ?? preset : preset)[protocol]
  const overrides = draft.endpointOverrides?.[protocol]
  return {
    ...value,
    host: overrides?.host ? value.host : defaults.host,
    port: overrides?.port ? value.port : String(defaults.port),
    tls: overrides?.tls ? value.tls : defaults.tls,
  }
}

function validOrganizationAddress(address: string, domain: string): boolean {
  // Match core's text and DNS hostname checks; leave mailbox naming and hosting
  // rules to the provider. The address never determines the server endpoint.
  const hostname = domain.replace(/\.+$/, '')
  return new TextEncoder().encode(address).length <= 320 && !/\p{Cc}/u.test(address)
    && hostname.length <= 253
    && hostname.split('.').every((label) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/i.test(label))
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

function connection(value: ConnectionDraft, protocol: Protocol, emailAddress: string): ConnectionConfig {
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
  const domain = emailDomain(emailAddress)
  if (!domain) invalid('emailAddress', t('accounts:invalidEmail'))
  const newProvider = account ? undefined : provider
  const domains = newProvider?.preset?.domains
  if (provider && domains && (domains === 'organization' ? !validOrganizationAddress(emailAddress, domain) : !domains.includes(domain))) {
    invalid('emailAddress', providerEmailRequired(provider))
  }
  const imap = connection(draftServer(draft, 'imap', newProvider), 'imap', emailAddress)
  const smtp = connection(draftServer(draft, 'smtp', newProvider), 'smtp', emailAddress)
  const authentication: ConnectionInput['authentication'] = account?.authentication.kind === 'oauth2'
    ? { kind: 'oauth2', configuration: account.authentication.configuration }
    : draft.mode === 'shared'
    ? { kind: 'password', mode: 'shared', password: secret(draft.sharedPassword, 'sharedPassword') }
    : { kind: 'password', mode: 'separate', imapPassword: secret(draft.imapPassword, 'imapPassword'), smtpPassword: secret(draft.smtpPassword, 'smtpPassword') }
  const route = draftToRoute(draft)
  return { emailAddress, providerId: account ? account.providerId : provider?.id ?? null, imap, smtp, authentication, route }
}

/** The network route a draft describes. Sign-in has a route but no servers or address yet. */
export function draftToRoute(draft: AccountDraft): RouteInput {
  let route: RouteInput = { kind: draft.routeKind === 'inherit' ? 'inherit' : 'direct' }
  if (draft.routeKind === 'custom') {
    if (draft.proxyAuth && !draft.proxyUsername.trim()) invalid('proxyUsername', t('accounts:proxyUsernameRequired'))
    route = {
      kind: 'custom', proxyKind: draft.proxyKind,
      host: host(draft.proxyHost, 'proxyHost'), port: port(draft.proxyPort, 'proxyPort'), dns: 'proxy',
      imageProxyDns: draft.imageProxyDns, confirmImageProxyDns: draft.confirmImageProxyDns,
      authentication: draft.proxyAuth ? { username: draft.proxyUsername.trim(), password: secret(draft.proxyPassword, 'proxyPassword') } : null,
    }
  }
  return route
}
