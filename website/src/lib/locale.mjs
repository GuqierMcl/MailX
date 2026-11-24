/**
 * The site's languages, shared by the pages, the Astro config and the Worker.
 * Directory names, URLs, the cookie and `lang` attributes all use these values.
 * @typedef {'zh-CN' | 'en'} Locale
 */

/** @type {readonly Locale[]} */
export const LOCALES = ['zh-CN', 'en']

/** @type {Locale} */
export const DEFAULT_LOCALE = 'zh-CN'

export const LANGUAGE_COOKIE = 'mailx-lang'

/**
 * Inline script for every page: reading a page counts as choosing its
 * language, which the Worker uses for addresses without one (worker/index.ts).
 */
export const setLanguageCookie = `document.cookie='${LANGUAGE_COOKIE}='+document.documentElement.lang+';path=/;max-age=31536000;samesite=lax'+(location.protocol==='https:'?';secure':'')`
