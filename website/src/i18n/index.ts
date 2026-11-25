import { LOCALES, type Locale } from '../lib/locale.mjs'
import en from './en'
import zhCN, { type Strings } from './zh-CN'

export { LOCALES, type Locale }

export const LOCALE_LABELS: Record<Locale, string> = { 'zh-CN': '简体中文', en: 'English' }
/** For the header's language switch, where space is short. */
export const LOCALE_SHORT: Record<Locale, string> = { 'zh-CN': '中文', en: 'EN' }
export const STRINGS: Record<Locale, Strings> = { 'zh-CN': zhCN, en }

/** Every page lives under its language: `/zh-CN/download/`, `/en/download/`. */
export function localePath(locale: Locale, path: string): string {
  return `/${locale}${path}`
}

export function otherLocale(locale: Locale): Locale {
  return locale === 'zh-CN' ? 'en' : 'zh-CN'
}
