import { t } from '@/i18n'
import type { IconSvgElement } from '@hugeicons/react'
import { Mail01Icon } from '@hugeicons/core-free-icons'
import qqmailLogo from '@/assets/providers/qqmail.svg'
import netease163Logo from '@/assets/providers/netease-163.svg'
import netease126Logo from '@/assets/providers/netease-126.svg'
import neteaseYeahLogo from '@/assets/providers/netease-yeah.svg'
import mail139Logo from '@/assets/providers/mail-139.svg'
import type { ConnectionConfig } from '@/generated/accounts'

export type ProviderPreset = {
  domains: readonly string[]
  imap: Omit<ConnectionConfig, 'username'>
  smtp: Omit<ConnectionConfig, 'username'>
}

/** Guidance for a preset that signs in with a client authorization code. */
export type ProviderSetup = {
  title: string
  description: string
  emailHint: string
  emailPlaceholder: string
  emailRequired: string
  authorizationCodeHint: string
  rangeHint?: string
  authenticationHelp: string
  providerPolicyHelp?: string
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
  /** A bundled logo URL, e.g. an imported local SVG; `icon` is the fallback. */
  logo?: string
  /** The provider's mark; the generic mail icon stands in until it has one. */
  icon: IconSvgElement
  /** Defaults for a new account only; saved connections always keep their values. */
  preset?: ProviderPreset
  /** Localized guidance; its presence enables the compact authorization-code form. */
  setup?: ProviderSetup
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
  get setup() {
    return {
      title: t('accounts:qqSetupTitle'),
      description: t('accounts:qqSetupDescription'),
      emailHint: t('accounts:qqEmailHint'),
      emailPlaceholder: 'you@qq.com',
      emailRequired: t('accounts:qqEmailRequired'),
      authorizationCodeHint: t('accounts:qqAuthorizationCodeHint'),
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
    get setup() {
      return {
        title: t('accounts:neteaseSetupTitle', { brand }),
        description: t('accounts:neteaseSetupDescription'),
        emailHint: t('accounts:neteaseEmailHint', { domain }),
        emailPlaceholder: `you@${domain}`,
        emailRequired: t('accounts:providerEmailRequired', { domains: `@${domain}` }),
        authorizationCodeHint: t('accounts:neteaseAuthorizationCodeHint'),
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
  get setup() {
    return {
      title: t('accounts:mail139SetupTitle'),
      description: t('accounts:mail139SetupDescription'),
      emailHint: t('accounts:mail139EmailHint'),
      emailPlaceholder: 'you@139.com',
      emailRequired: t('accounts:providerEmailRequired', { domains: '@139.com' }),
      authorizationCodeHint: t('accounts:mail139AuthorizationCodeHint'),
      authenticationHelp: t('accounts:mail139AuthenticationHelp'),
    }
  },
}

export const PROVIDERS: MailProvider[] = [QQ_PROVIDER, NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER, MAIL_139_PROVIDER, GENERIC_PROVIDER]

/** Address errors use only the preset's reviewed domains and localized guidance. */
export function providerEmailRequired(provider: MailProvider): string {
  if (!provider.preset) return t('accounts:invalidEmail')
  return provider.setup?.emailRequired ?? t('accounts:providerEmailRequired', {
    domains: provider.preset.domains.map((domain) => `@${domain}`).join(' / '),
  })
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
