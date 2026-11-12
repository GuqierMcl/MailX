import { i18n, t } from '@/i18n'
import type { TranslationProviderId } from '@/generated/accounts'

/** The languages a message can be translated into; the core accepts exactly these. */
export const TRANSLATION_TARGETS = [
  'zh-Hans', 'zh-Hant', 'en', 'ja', 'ko', 'fr', 'de', 'es', 'pt', 'it', 'ru', 'nl', 'pl', 'tr',
  'ar', 'id', 'uk', 'sv', 'da', 'fi', 'cs', 'el', 'hu', 'ro', 'bg', 'nb',
] as const

/** Base language codes offered for the "never suggest" and "always translate" lists. */
export const COMMON_LANGUAGES = [
  'en', 'zh', 'ja', 'ko', 'fr', 'de', 'es', 'pt', 'it', 'ru', 'nl', 'pl', 'tr', 'ar', 'id', 'uk',
  'sv', 'da', 'fi', 'cs', 'el', 'hu', 'ro', 'bg', 'nb', 'vi', 'th', 'hi', 'he',
] as const

/** The two services that need no key, in the order the settings list them. */
export const KEYLESS_PROVIDERS = ['microsoftFree', 'googleFree'] as const satisfies readonly TranslationProviderId[]
export const KEYED_PROVIDERS = ['deepl', 'microsoft', 'google'] as const satisfies readonly TranslationProviderId[]

export const needsKey = (provider: TranslationProviderId) => !(KEYLESS_PROVIDERS as readonly string[]).includes(provider)

/** Who receives the text of a keyless service, for the consent sentence. */
export const KEYLESS_COMPANY: Record<(typeof KEYLESS_PROVIDERS)[number], string> = { microsoftFree: 'Microsoft', googleFree: 'Google' }

/** Display names; the two keyless services follow the interface language, the keyed ones are brand names. Read at render time. */
export const TRANSLATION_PROVIDER_NAMES: Record<TranslationProviderId, string> = {
  deepl: 'DeepL',
  microsoft: 'Microsoft Translator',
  google: 'Google Cloud Translation',
  get microsoftFree() { return t('settings:translationProviderMicrosoftFree') },
  get googleFree() { return t('settings:translationProviderGoogleFree') },
}

function displayName(code: string, locale: string): string | null {
  try {
    const name = new Intl.DisplayNames([locale], { type: 'language' }).of(code)
    return name && name !== code ? name : null
  } catch {
    return null
  }
}

/** A language's name in the interface language, falling back to its code. */
export function languageName(code: string): string {
  return displayName(code, i18n.language) ?? code
}

/** A target language written in itself, so it can be found whatever the interface language. */
export function nativeLanguageName(code: string): string {
  const name = displayName(code, code)
  return name ? name.charAt(0).toLocaleUpperCase(code) + name.slice(1) : code
}

/** The base code of a target (`zh-Hans` is `zh`), as the never / always lists hold them. */
export const baseLanguage = (code: string) => code.split('-')[0].toLowerCase()
