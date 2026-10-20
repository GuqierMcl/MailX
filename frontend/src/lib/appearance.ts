import type { AppearanceSettings, FontSize, ThemeMode } from '@/generated/accounts'

export const DEFAULT_APPEARANCE: AppearanceSettings = {
  theme: 'system',
  accentColor: '',
  interfaceFont: '',
  fontSize: 'default',
}

/** Used only when the user leaves the interface font empty. Keep in sync with `index.css`. */
export const DEFAULT_INTERFACE_FONTS = [
  'Inter Variable',
  'PingFang SC',
  'Hiragino Sans GB',
  'Microsoft YaHei UI',
  'Microsoft YaHei',
  'Noto Sans CJK SC',
  'Noto Sans SC',
  'Source Han Sans SC',
  'system-ui',
  'sans-serif',
]

/** `''` keeps the brand's graphite tokens from `index.css`. */
export const ACCENT_PRESETS = [
  { value: '', label: '石墨' },
  { value: '#2563eb', label: '蓝' },
  { value: '#7c3aed', label: '紫' },
  { value: '#0d9488', label: '青' },
  { value: '#16a34a', label: '绿' },
  { value: '#ea580c', label: '橙' },
  { value: '#e11d48', label: '玫红' },
] as const

const ROOT_FONT_SIZE: Record<FontSize, string> = { small: '15px', default: '', large: '17px' }

const GENERIC_FAMILIES = new Set([
  'serif', 'sans-serif', 'monospace', 'cursive', 'fantasy', 'system-ui', 'math', 'emoji', 'fangsong',
  'ui-serif', 'ui-sans-serif', 'ui-monospace', 'ui-rounded',
])

/** Mirrors core's normalization: split on commas, trim, unquote, drop empty entries. */
export function parseFontFamilies(value: string): string[] {
  return value
    .split(',')
    .map((family) => family.trim().replace(/^(["'])(.*)\1$/, '$2').trim())
    .filter(Boolean)
}

/**
 * Builds the CSS `font-family` value. A non-empty user list is used as is, in
 * order; only an empty one falls back to the defaults. A trailing generic
 * family keeps text sans-serif when none of the listed fonts is installed.
 */
export function fontFamilyCss(interfaceFont: string): string {
  const user = parseFontFamilies(interfaceFont).filter((family) => !/["\\]/.test(family))
  const families = user.length ? user : DEFAULT_INTERFACE_FONTS
  const css = families.map((family) => (GENERIC_FAMILIES.has(family.toLowerCase()) ? family : `"${family}"`))
  if (!families.some((family) => GENERIC_FAMILIES.has(family.toLowerCase()))) css.push('sans-serif')
  return css.join(', ')
}

function relativeLuminance(hex: string) {
  const channel = (offset: number) => {
    const value = parseInt(hex.slice(offset, offset + 2), 16) / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
}

/** Picks whichever of white or near-black text contrasts more with the accent. */
export function accentForeground(hex: string) {
  const luminance = relativeLuminance(hex)
  const onWhite = 1.05 / (luminance + 0.05)
  const onNearBlack = (luminance + 0.05) / (relativeLuminance('#171717') + 0.05)
  return onWhite >= onNearBlack ? '#ffffff' : '#171717'
}

const ACCENT_PROPERTIES = ['--primary', '--ring', '--sidebar-primary', '--sidebar-ring']
const ACCENT_FOREGROUND_PROPERTIES = ['--primary-foreground', '--sidebar-primary-foreground']

let stopFollowingSystem: (() => void) | undefined

function applyTheme(theme: ThemeMode) {
  stopFollowingSystem?.()
  stopFollowingSystem = undefined
  const query = window.matchMedia('(prefers-color-scheme: dark)')
  const update = () =>
    document.documentElement.classList.toggle('dark', theme === 'dark' || (theme === 'system' && query.matches))
  update()
  if (theme !== 'system') return
  query.addEventListener('change', update)
  stopFollowingSystem = () => query.removeEventListener('change', update)
}

/**
 * Applies appearance through CSSOM custom properties on `<html>`, which the
 * `style-src 'self'` CSP allows, and which override both light and dark tokens.
 */
export function applyAppearance(appearance: AppearanceSettings) {
  const style = document.documentElement.style
  applyTheme(appearance.theme)
  if (/^#[0-9a-f]{6}$/i.test(appearance.accentColor)) {
    const foreground = accentForeground(appearance.accentColor)
    for (const property of ACCENT_PROPERTIES) style.setProperty(property, appearance.accentColor)
    for (const property of ACCENT_FOREGROUND_PROPERTIES) style.setProperty(property, foreground)
  } else {
    for (const property of [...ACCENT_PROPERTIES, ...ACCENT_FOREGROUND_PROPERTIES]) style.removeProperty(property)
  }
  style.setProperty('--app-font-sans', fontFamilyCss(appearance.interfaceFont))
  style.fontSize = ROOT_FONT_SIZE[appearance.fontSize] ?? ''
}

const CACHE_KEY = 'mailx.appearance'
/** Pre-`settings.json` theme preference, imported once and then removed. */
const LEGACY_THEME_KEY = 'mailx.theme'

const THEMES: readonly string[] = ['system', 'light', 'dark'] satisfies ThemeMode[]
const FONT_SIZES: readonly string[] = ['small', 'default', 'large'] satisfies FontSize[]

/**
 * The core's `settings.json` is the source of truth; this copy only lets the
 * first frame render with the last known appearance before IPC answers.
 */
export function cachedAppearance(): AppearanceSettings {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(CACHE_KEY) ?? 'null')
    if (value && typeof value === 'object') {
      const cached = value as Record<string, unknown>
      return {
        theme: THEMES.includes(cached.theme as string) ? (cached.theme as ThemeMode) : 'system',
        accentColor: typeof cached.accentColor === 'string' ? cached.accentColor : '',
        interfaceFont: typeof cached.interfaceFont === 'string' ? cached.interfaceFont : '',
        fontSize: FONT_SIZES.includes(cached.fontSize as string) ? (cached.fontSize as FontSize) : 'default',
      }
    }
  } catch {
    // A corrupt cache only costs the first frame; the core answer replaces it.
  }
  return { ...DEFAULT_APPEARANCE, theme: legacyTheme() ?? 'system' }
}

export function cacheAppearance(appearance: AppearanceSettings) {
  localStorage.setItem(CACHE_KEY, JSON.stringify(appearance))
}

export function legacyTheme(): ThemeMode | null {
  const value = localStorage.getItem(LEGACY_THEME_KEY)
  return value === 'light' || value === 'dark' ? value : null
}

export function clearLegacyTheme() {
  localStorage.removeItem(LEGACY_THEME_KEY)
}
