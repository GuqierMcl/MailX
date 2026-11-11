import { i18n } from '@/i18n'
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

/** Brand names are not translated. */
export const TRANSLATION_PROVIDER_NAMES: Record<TranslationProviderId, string> = {
  deepl: 'DeepL',
  microsoft: 'Microsoft Translator',
  google: 'Google Cloud Translation',
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
