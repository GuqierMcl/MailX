/**
 * Light or dark, shared with the Starlight docs through its own storage key:
 * `light`, `dark`, or empty to follow the system.
 */
export const THEME_KEY = 'starlight-theme'

/**
 * Inline script for <head>, so the first paint already has the right colours.
 * Screenshots choose their file by media query, so a chosen theme rewrites the
 * query; Screenshot.astro calls `mxTheme.apply()` right after each picture.
 */
export const themeScript = `(() => {
  const system = matchMedia('(prefers-color-scheme: dark)')
  const stored = () => { try { const v = localStorage.getItem('${THEME_KEY}'); return v === 'light' || v === 'dark' ? v : '' } catch { return '' } }
  let chosen = stored()
  const current = () => chosen || (system.matches ? 'dark' : 'light')
  const apply = () => {
    const theme = current()
    document.documentElement.dataset.theme = theme
    document.querySelectorAll('source[data-dark]').forEach((source) => { source.media = theme === 'dark' ? 'all' : 'not all' })
    document.querySelectorAll('.theme-toggle').forEach((button) => { button.setAttribute('aria-pressed', String(theme === 'dark')) })
  }
  const toggle = () => {
    const next = current() === 'dark' ? 'light' : 'dark'
    // Choosing what the system already uses goes back to following it.
    chosen = next === (system.matches ? 'dark' : 'light') ? '' : next
    try { localStorage.setItem('${THEME_KEY}', chosen) } catch {}
    apply()
  }
  system.addEventListener('change', apply)
  addEventListener('storage', () => { chosen = stored(); apply() })
  addEventListener('pageshow', (event) => { if (event.persisted) { chosen = stored(); apply() } })
  window.mxTheme = { apply, toggle }
  apply()
})()`
