import { t } from '@/i18n'
import type { IconSvgElement } from '@hugeicons/react'
import { Mail01Icon } from '@hugeicons/core-free-icons'

/**
 * A mail provider MailX knows how to add, and how to label afterwards. The
 * add-account cards, the account manager and the connection dialog all read
 * this list, so a new provider is one entry here plus its connection step.
 */
export type MailProvider = {
  /** Stored as the account's `providerId`; `null` is a manually configured server. */
  id: string | null
  name: string
  /** The address suffix people recognize, e.g. `gmail.com`. */
  domain: string
  /** The provider's mark; the generic mail icon stands in until it has one. */
  icon: IconSvgElement
}

export const GENERIC_PROVIDER: MailProvider = {
  id: null,
  name: 'IMAP/SMTP',
  // Works with any server, so it names no single suffix.
  get domain() { return t('accounts:otherProvider') },
  icon: Mail01Icon,
}

export const PROVIDERS: MailProvider[] = [GENERIC_PROVIDER]

/**
 * The provider an account was added with. An id this build does not know
 * (e.g. written by a newer version) still gets a label instead of failing.
 */
export function providerOf(providerId: string | null): MailProvider {
  return PROVIDERS.find((provider) => provider.id === providerId)
    ?? { ...GENERIC_PROVIDER, id: providerId, name: providerId ?? GENERIC_PROVIDER.name }
}
