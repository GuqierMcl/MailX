import { t } from '@/i18n'
import type { IconSvgElement } from '@hugeicons/react'
import { Mail01Icon } from '@hugeicons/core-free-icons'
import qqmailLogo from '@/assets/providers/qqmail.png'
import type { ConnectionConfig } from '@/generated/accounts'

export type ProviderPreset = {
  domains: readonly string[]
  imap: Omit<ConnectionConfig, 'username'>
  smtp: Omit<ConnectionConfig, 'username'>
}

/**
 * A mail provider MailX knows how to add, and how to label afterwards. The
 * add-account list, the account manager and the connection dialog all read
 * this list, so a new provider is one entry here plus its connection step.
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
}

export const PROVIDERS: MailProvider[] = [QQ_PROVIDER, GENERIC_PROVIDER]

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
