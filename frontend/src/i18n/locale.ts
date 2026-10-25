import { i18n } from './index'

export type SupportedLocale = 'zh-CN' | 'en'
export type LanguagePreference = 'system' | SupportedLocale
const CACHE_KEY = 'mailx.language'

export function systemLocale(languages: readonly string[] = navigator.languages): SupportedLocale {
  const language = languages[0]?.replaceAll('_', '-').toLowerCase() ?? ''
  return language === 'zh' || language.startsWith('zh-') ? 'zh-CN' : 'en'
}

export function cachedLanguage(): { language: LanguagePreference; effectiveLocale: SupportedLocale } {
  try {
    const cached: unknown = JSON.parse(localStorage.getItem(CACHE_KEY) ?? 'null')
    if (cached && typeof cached === 'object') {
      const { language, effectiveLocale } = cached as Record<string, unknown>
      if ((language === 'system' || language === 'zh-CN' || language === 'en') &&
          (effectiveLocale === 'zh-CN' || effectiveLocale === 'en')) return { language, effectiveLocale }
    }
  } catch { /* A first-frame cache is optional. The host remains authoritative. */ }
  return { language: 'system', effectiveLocale: systemLocale() }
}

export function applyLanguage(locale: SupportedLocale, preference?: LanguagePreference) {
  if (i18n.language !== locale) void i18n.changeLanguage(locale)
  document.documentElement.lang = locale
  if (preference) {
    try { localStorage.setItem(CACHE_KEY, JSON.stringify({ language: preference, effectiveLocale: locale })) } catch { /* Optional cache. */ }
  }
}

export function initializeLanguage() {
  applyLanguage(cachedLanguage().effectiveLocale)
}
