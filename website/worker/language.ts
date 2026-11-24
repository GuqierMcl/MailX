import { DEFAULT_LOCALE, LANGUAGE_COOKIE, LOCALES, type Locale } from '../src/lib/locale.mjs'

export function cookieLocale(cookie: string | null): Locale | null {
  for (const part of (cookie ?? '').split(';')) {
    const [name, value] = part.trim().split('=')
    const locale = LOCALES.find((item) => item === value)
    if (name === LANGUAGE_COOKIE && locale) return locale
  }
  return null
}

/** The highest-weighted language in Accept-Language, or null when there is none. */
export function preferredLanguage(header: string | null): string | null {
  let best: { tag: string; q: number } | null = null
  for (const part of (header ?? '').split(',')) {
    const [rawTag, ...params] = part.trim().split(';')
    const tag = rawTag.trim().toLowerCase()
    if (!tag || tag === '*') continue
    const weight = params.map((param) => param.trim()).find((param) => param.startsWith('q='))
    const q = weight ? Number(weight.slice(2)) : 1
    if (!Number.isFinite(q) || q <= 0) continue
    // Equal weights keep header order: the first one listed is preferred.
    if (!best || q > best.q) best = { tag, q }
  }
  return best?.tag ?? null
}

export function chooseLocale(request: Request): Locale {
  const fromCookie = cookieLocale(request.headers.get('Cookie'))
  if (fromCookie) return fromCookie
  const language = preferredLanguage(request.headers.get('Accept-Language'))
  if (language === null) return DEFAULT_LOCALE
  return language === 'zh' || language.startsWith('zh-') ? 'zh-CN' : 'en'
}

/**
 * Where an address without a language leads. The result always starts with
 * one of our own languages, so it can never point at another site.
 */
export function localizedPath(locale: Locale, pathname: string): string {
  const path = pathname.endsWith('/') ? pathname : `${pathname}/`
  return `/${locale}${path === '/privacy/' ? '/docs/privacy/' : path}`
}
