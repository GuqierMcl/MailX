import { t } from '@/i18n'
import type { IconSvgElement } from '@hugeicons/react'
import { Mail01Icon } from '@hugeicons/core-free-icons'
import qqmailLogo from '@/assets/providers/qqmail.png'
import netease163Logo from '@/assets/providers/netease-163.png'
import netease126Logo from '@/assets/providers/netease-126.png'
import neteaseYeahLogo from '@/assets/providers/netease-yeah.png'
import mail139Logo from '@/assets/providers/mail-139.png'
import sinaMailLogo from '@/assets/providers/sina-mail.png'
import feishuLogo from '@/assets/providers/feishu.png'
import larkLogo from '@/assets/providers/lark.png'
import icloudMailLogo from '@/assets/providers/icloud-mail.png'
import gmailLogo from '@/assets/providers/gmail.svg'
import outlookLogo from '@/assets/providers/outlook.svg'
import microsoftLogo from '@/assets/providers/microsoft.svg'
import exchangeLogo from '@/assets/providers/exchange.png'
import type { ConnectionConfig } from '@/generated/accounts'

type PresetServers = {
  imap: Omit<ConnectionConfig, 'username'>
  smtp: Omit<ConnectionConfig, 'username'>
}

export type ProviderPreset = PresetServers & (
  | { domains: readonly string[]; serversByDomain?: Readonly<Partial<Record<string, PresetServers>>> }
  /** Explicit selection for organization domains, with fixed provider endpoints. */
  | { domains: 'organization'; serversByDomain?: never }
)

/** Guidance for a preset that signs in with a provider-issued client secret. */
export type ProviderSetup = {
  credentialKind: 'authorizationCode' | 'appPassword'
  title: string
  description: string
  emailHint: string
  emailPlaceholder: string
  emailRequired: string
  credentialHint: string
  rangeHint?: string
  authenticationHelp: string
  providerPolicyHelp?: string
}

/**
 * Guidance for a provider that signs in through the browser. The user gives
 * only the address; servers and sign-in rules come from the host's reviewed
 * list, never from the page.
 */
export type SignInSetup = {
  title: string
  description: string
  emailHint: string
  emailPlaceholder: string
  /** Short help shown under the entry in the provider list. */
  help: string
}

/**
 * A mail provider MailX knows how to add, and how to label afterwards. The
 * add-account list, the account manager and the connection dialog all read
 * this list; presets and setup guidance stay with the provider's entry.
 */
export type MailProvider = {
  /** Stored as the account's `providerId`; `null` is a manually configured server. */
  id: string | null
  name: string
  /** The address suffix people recognize, e.g. `gmail.com`. */
  domain: string
  /** A short, localized explanation shown beside the provider's name. */
  description: string
  /** Alternate names and terms people may search for. */
  searchAliases?: readonly string[]
  /** A bundled logo URL, e.g. an imported local PNG; `icon` is the fallback. */
  logo?: string
  /**
   * `launcher`: the logo is an official app launcher icon, published as a
   * full-bleed square that platforms always show with rounded corners. The
   * file stays unmodified; only its display gets the same rounding.
   */
  logoShape?: 'launcher'
  /** The provider's mark; the generic mail icon stands in until it has one. */
  icon: IconSvgElement
  /** Defaults for a new account only; saved connections always keep their values. */
  preset?: ProviderPreset
  /** Localized guidance; its presence enables the compact credential form. */
  setup?: ProviderSetup
  /** Present for providers that sign in in the browser instead of with a password. */
  signIn?: SignInSetup
  /** The explicit self-hosted Exchange path; independent from browser OAuth. */
  connectionKind?: 'ews'
}

export const GENERIC_PROVIDER: MailProvider = {
  id: null,
  name: 'IMAP/SMTP',
  // Works with any server, so it names no single suffix.
  get domain() { return t('accounts:otherProvider') },
  get description() { return t('accounts:genericProviderDescription') },
  searchAliases: ['manual', 'custom', 'other', '手动', '自定义', '其他邮箱'],
  icon: Mail01Icon,
}

export const EXCHANGE_PROVIDER: MailProvider = {
  id: 'exchange-self-hosted',
  get name() { return t('accounts:exchangeProviderName') },
  get domain() { return t('accounts:exchangeProviderDomain') },
  get description() { return t('accounts:exchangeProviderDescription') },
  searchAliases: ['Exchange', 'EWS', 'NTLM', 'self-hosted', '自建', '企业邮箱'],
  logo: exchangeLogo,
  icon: Mail01Icon,
  connectionKind: 'ews',
}

const QQ_DOMAINS = ['qq.com', 'vip.qq.com', 'foxmail.com'] as const

export const QQ_PROVIDER: MailProvider = {
  id: 'qq',
  get name() { return t('accounts:qqProviderName') },
  domain: QQ_DOMAINS.join(' / '),
  get description() { return t('accounts:qqProviderDescription') },
  searchAliases: ['qq', 'foxmail', '腾讯', '腾讯邮箱', 'QQ邮箱', 'VIP邮箱'],
  logo: qqmailLogo,
  icon: Mail01Icon,
  preset: {
    domains: QQ_DOMAINS,
    imap: { host: 'imap.qq.com', port: 993, tls: 'implicit' },
    smtp: { host: 'smtp.qq.com', port: 465, tls: 'implicit' },
  },
  get setup(): ProviderSetup {
    return {
      credentialKind: 'authorizationCode',
      title: t('accounts:qqSetupTitle'),
      description: t('accounts:qqSetupDescription'),
      emailHint: t('accounts:qqEmailHint'),
      emailPlaceholder: 'you@qq.com',
      emailRequired: t('accounts:qqEmailRequired'),
      credentialHint: t('accounts:qqAuthorizationCodeHint'),
      rangeHint: t('accounts:qqRangeHint'),
      authenticationHelp: t('accounts:qqAuthenticationHelp'),
    }
  },
}

function neteaseProvider(id: string, domain: '163.com' | '126.com' | 'yeah.net', logo: string): MailProvider {
  const brand = domain === 'yeah.net' ? 'yeah.net' : domain.split('.')[0]
  return {
    id,
    get name() { return t('accounts:neteaseProviderName', { brand }) },
    domain,
    get description() { return t('accounts:neteaseProviderDescription', { domain }) },
    searchAliases: ['网易', '网易邮箱', 'NetEase', brand],
    logo,
    icon: Mail01Icon,
    preset: {
      domains: [domain],
      imap: { host: `imap.${domain}`, port: 993, tls: 'implicit' },
      smtp: { host: `smtp.${domain}`, port: 465, tls: 'implicit' },
    },
    get setup(): ProviderSetup {
      return {
        credentialKind: 'authorizationCode',
        title: t('accounts:neteaseSetupTitle', { brand }),
        description: t('accounts:neteaseSetupDescription'),
        emailHint: t('accounts:neteaseEmailHint', { domain }),
        emailPlaceholder: `you@${domain}`,
        emailRequired: t('accounts:providerEmailRequired', { domains: `@${domain}` }),
        credentialHint: t('accounts:neteaseAuthorizationCodeHint'),
        rangeHint: t('accounts:neteaseRangeHint'),
        authenticationHelp: t('accounts:neteaseAuthenticationHelp'),
        providerPolicyHelp: t('accounts:neteaseProviderPolicyHelp'),
      }
    },
  }
}

export const NETEASE_163_PROVIDER = neteaseProvider('netease163', '163.com', netease163Logo)
export const NETEASE_126_PROVIDER = neteaseProvider('netease126', '126.com', netease126Logo)
export const NETEASE_YEAH_PROVIDER = neteaseProvider('neteaseYeah', 'yeah.net', neteaseYeahLogo)

export const MAIL_139_PROVIDER: MailProvider = {
  id: '139',
  get name() { return t('accounts:mail139ProviderName') },
  domain: '139.com',
  get description() { return t('accounts:mail139ProviderDescription') },
  searchAliases: ['139', '139邮箱', '中国移动', '移动邮箱', 'China Mobile', 'CMCC'],
  logo: mail139Logo,
  icon: Mail01Icon,
  preset: {
    domains: ['139.com'],
    imap: { host: 'imap.139.com', port: 993, tls: 'implicit' },
    smtp: { host: 'smtp.139.com', port: 465, tls: 'implicit' },
  },
  get setup(): ProviderSetup {
    return {
      credentialKind: 'authorizationCode',
      title: t('accounts:mail139SetupTitle'),
      description: t('accounts:mail139SetupDescription'),
      emailHint: t('accounts:mail139EmailHint'),
      emailPlaceholder: 'you@139.com',
      emailRequired: t('accounts:providerEmailRequired', { domains: '@139.com' }),
      credentialHint: t('accounts:mail139AuthorizationCodeHint'),
      authenticationHelp: t('accounts:mail139AuthenticationHelp'),
    }
  },
}

const ICLOUD_DOMAINS = ['icloud.com', 'me.com', 'mac.com'] as const

export const ICLOUD_PROVIDER: MailProvider = {
  id: 'icloud',
  get name() { return t('accounts:icloudProviderName') },
  domain: ICLOUD_DOMAINS.join(' / '),
  get description() { return t('accounts:icloudProviderDescription') },
  searchAliases: ['iCloud', 'Apple', '苹果', '苹果邮箱'],
  logo: icloudMailLogo,
  icon: Mail01Icon,
  preset: {
    domains: ICLOUD_DOMAINS,
    imap: { host: 'imap.mail.me.com', port: 993, tls: 'implicit' },
    smtp: { host: 'smtp.mail.me.com', port: 587, tls: 'startTls' },
  },
  get setup(): ProviderSetup {
    return {
      credentialKind: 'appPassword',
      title: t('accounts:icloudSetupTitle'),
      description: t('accounts:icloudSetupDescription'),
      emailHint: t('accounts:icloudEmailHint'),
      emailPlaceholder: 'you@icloud.com',
      emailRequired: t('accounts:icloudEmailRequired'),
      credentialHint: t('accounts:icloudAppPasswordHint'),
      authenticationHelp: t('accounts:icloudAuthenticationHelp'),
    }
  },
}

const GMAIL_DOMAINS = ['gmail.com', 'googlemail.com'] as const

export const GMAIL_PROVIDER: MailProvider = {
  id: 'gmail',
  get name() { return t('accounts:gmailProviderName') },
  domain: GMAIL_DOMAINS.join(' / '),
  get description() { return t('accounts:gmailProviderDescription') },
  searchAliases: ['Gmail', 'Google', '谷歌', '谷歌邮箱'],
  logo: gmailLogo,
  icon: Mail01Icon,
  preset: {
    domains: GMAIL_DOMAINS,
    imap: { host: 'imap.gmail.com', port: 993, tls: 'implicit' },
    smtp: { host: 'smtp.gmail.com', port: 465, tls: 'implicit' },
  },
  get setup(): ProviderSetup {
    return {
      credentialKind: 'appPassword',
      title: t('accounts:gmailSetupTitle'),
      description: t('accounts:gmailSetupDescription'),
      emailHint: t('accounts:gmailEmailHint'),
      emailPlaceholder: 'you@gmail.com',
      emailRequired: t('accounts:gmailEmailRequired'),
      credentialHint: t('accounts:gmailAppPasswordHint'),
      authenticationHelp: t('accounts:gmailAuthenticationHelp'),
    }
  },
}

const SINA_COM_SERVERS: PresetServers = {
  imap: { host: 'imap.sina.com', port: 993, tls: 'implicit' },
  smtp: { host: 'smtp.sina.com', port: 465, tls: 'implicit' },
}
const SINA_SERVERS_BY_DOMAIN = {
  'sina.com': SINA_COM_SERVERS,
  'sina.cn': {
    imap: { host: 'imap.sina.cn', port: 993, tls: 'implicit' },
    smtp: { host: 'smtp.sina.cn', port: 465, tls: 'implicit' },
  },
} satisfies Record<string, PresetServers>
const SINA_DOMAINS = Object.keys(SINA_SERVERS_BY_DOMAIN)

/** Display-only rounding for official launcher icons; other logos keep their own shape. */
export function logoShapeClass(provider: Pick<MailProvider, 'logoShape'>): string | undefined {
  // Close to the corner radius iOS and Android apply to launcher icons.
  return provider.logoShape === 'launcher' ? 'rounded-[22.5%]' : undefined
}

export const SINA_PROVIDER: MailProvider = {
  id: 'sina',
  get name() { return t('accounts:sinaProviderName') },
  domain: SINA_DOMAINS.join(' / '),
  get description() { return t('accounts:sinaProviderDescription') },
  searchAliases: ['Sina', '新浪', '新浪邮箱', '新浪免费邮'],
  logo: sinaMailLogo,
  logoShape: 'launcher',
  icon: Mail01Icon,
  preset: { domains: SINA_DOMAINS, ...SINA_COM_SERVERS, serversByDomain: SINA_SERVERS_BY_DOMAIN },
  get setup(): ProviderSetup {
    return {
      credentialKind: 'authorizationCode',
      title: t('accounts:sinaSetupTitle'),
      description: t('accounts:sinaSetupDescription'),
      emailHint: t('accounts:sinaEmailHint'),
      emailPlaceholder: 'you@sina.com',
      emailRequired: t('accounts:sinaEmailRequired'),
      credentialHint: t('accounts:sinaAuthorizationCodeHint'),
      authenticationHelp: t('accounts:sinaAuthenticationHelp'),
    }
  },
}

// An organization's address may look the same on either service. Only the
// explicitly selected preset determines where its dedicated password is used.
const HOSTED_MAIL = {
  feishu: { serverDomain: 'feishu.cn', logo: feishuLogo, searchAliases: ['Feishu', '飞书', '飞书邮箱', '企业邮箱'] },
  lark: { serverDomain: 'larksuite.com', logo: larkLogo, searchAliases: ['Lark Suite', '飞书国际版', '国际版飞书', '企业邮箱'] },
} as const

function hostedMailProvider(id: keyof typeof HOSTED_MAIL): MailProvider {
  const { serverDomain, logo, searchAliases } = HOSTED_MAIL[id]
  return {
    id,
    get name() { return t(`accounts:${id}ProviderName`) },
    get domain() { return t('accounts:organizationDomain') },
    get description() { return t(`accounts:${id}ProviderDescription`) },
    searchAliases: [...searchAliases, serverDomain],
    logo,
    icon: Mail01Icon,
    preset: {
      domains: 'organization',
      imap: { host: `imap.${serverDomain}`, port: 993, tls: 'implicit' },
      smtp: { host: `smtp.${serverDomain}`, port: 465, tls: 'implicit' },
    },
    get setup(): ProviderSetup {
      return {
        credentialKind: 'appPassword',
        title: t(`accounts:${id}SetupTitle`),
        description: t(`accounts:${id}SetupDescription`),
        emailHint: t(`accounts:${id}EmailHint`),
        emailPlaceholder: 'you@company.com',
        emailRequired: t(`accounts:${id}EmailRequired`),
        credentialHint: t(`accounts:${id}AppPasswordHint`),
        authenticationHelp: t(`accounts:${id}AuthenticationHelp`),
        providerPolicyHelp: t(`accounts:${id}ProviderPolicyHelp`),
      }
    },
  }
}

export const FEISHU_PROVIDER = hostedMailProvider('feishu')
export const LARK_PROVIDER = hostedMailProvider('lark')

// The ids are the host's reviewed provider ids and are also stored as the
// account's `providerId`. Outlook uses Microsoft's product icon; the work entry
// uses the Microsoft logo, as Microsoft's sign-in guidance asks for work or
// school accounts (see assets/providers/README.md).
export const MICROSOFT_PERSONAL_PROVIDER: MailProvider = {
  id: 'microsoft-personal',
  get name() { return t('accounts:microsoftPersonalProviderName') },
  get domain() { return 'outlook.com / hotmail.com / live.com' },
  get description() { return t('accounts:microsoftPersonalProviderDescription') },
  searchAliases: ['Outlook', 'Hotmail', 'Live', 'MSN', 'Microsoft', '微软'],
  logo: outlookLogo,
  icon: Mail01Icon,
  get signIn(): SignInSetup {
    return {
      title: t('accounts:microsoftPersonalSignInTitle'),
      description: t('accounts:microsoftPersonalSignInDescription'),
      emailHint: t('accounts:microsoftPersonalEmailHint'),
      emailPlaceholder: 'you@outlook.com',
      help: t('accounts:microsoftHelp'),
    }
  },
}

export const MICROSOFT_WORK_PROVIDER: MailProvider = {
  id: 'microsoft-work',
  get name() { return t('accounts:microsoftWorkProviderName') },
  get domain() { return t('accounts:microsoftWorkDomain') },
  get description() { return t('accounts:microsoftWorkProviderDescription') },
  searchAliases: ['Exchange', 'Office 365', 'Microsoft 365', 'Microsoft', '微软', '企业邮箱'],
  logo: microsoftLogo,
  icon: Mail01Icon,
  get signIn(): SignInSetup {
    return {
      title: t('accounts:microsoftWorkSignInTitle'),
      description: t('accounts:microsoftWorkSignInDescription'),
      emailHint: t('accounts:microsoftWorkEmailHint'),
      emailPlaceholder: 'you@company.com',
      help: t('accounts:microsoftHelp'),
    }
  },
}

export const PROVIDERS: MailProvider[] = [
  GMAIL_PROVIDER, ICLOUD_PROVIDER, MICROSOFT_PERSONAL_PROVIDER, MICROSOFT_WORK_PROVIDER,
  QQ_PROVIDER, NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER, MAIL_139_PROVIDER,
  SINA_PROVIDER, FEISHU_PROVIDER, LARK_PROVIDER, EXCHANGE_PROVIDER, GENERIC_PROVIDER,
]

const CREDENTIAL_TEXT_KEYS = {
  password: {
    shared: 'accounts:sharedPasswordLabel', incoming: 'accounts:incomingPassword', outgoing: 'accounts:outgoingPassword',
    placeholder: 'accounts:enterPassword', help: 'accounts:providerPolicyHelpLink',
  },
  authorizationCode: {
    shared: 'accounts:authorizationCode', incoming: 'accounts:incomingAuthorizationCode', outgoing: 'accounts:outgoingAuthorizationCode',
    placeholder: 'accounts:enterAuthorizationCode', help: 'accounts:getAuthorizationCode',
  },
  appPassword: {
    shared: 'accounts:appPassword', incoming: 'accounts:incomingAppPassword', outgoing: 'accounts:outgoingAppPassword',
    placeholder: 'accounts:enterAppPassword', help: 'accounts:getAppPassword',
  },
} as const

/** Shared form and recovery copy follows credential metadata, not provider names. */
export function providerCredentialText(setup?: ProviderSetup) {
  const keys = CREDENTIAL_TEXT_KEYS[setup?.credentialKind ?? 'password']
  return { shared: t(keys.shared), incoming: t(keys.incoming), outgoing: t(keys.outgoing), placeholder: t(keys.placeholder), help: t(keys.help) }
}

/** Address errors use only the preset's reviewed domains and localized guidance. */
export function providerEmailRequired(provider: MailProvider): string {
  const domains = provider.preset?.domains
  return provider.setup?.emailRequired ?? (domains && domains !== 'organization'
    ? t('accounts:providerEmailRequired', { domains: domains.map((domain) => `@${domain}`).join(' / ') })
    : t('accounts:invalidEmail'))
}

/** Local filtering keeps the registry's order and follows the current UI language. */
export function filterProviders(providers: readonly MailProvider[], query: string): MailProvider[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  return providers.filter((provider) => {
    const searchable = [provider.name, provider.domain, provider.description, ...(provider.searchAliases ?? [])]
      .join(' ').toLowerCase()
    return terms.every((term) => searchable.includes(term))
  })
}

/**
 * The provider an account was added with. An id this build does not know
 * (e.g. written by a newer version) still gets a label instead of failing.
 */
export function providerOf(providerId: string | null): MailProvider {
  return PROVIDERS.find((provider) => provider.id === providerId)
    ?? { ...GENERIC_PROVIDER, id: providerId, name: providerId ?? GENERIC_PROVIDER.name }
}
