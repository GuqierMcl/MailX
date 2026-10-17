import { useEffect, useState } from 'react'

export type ThemePreference = 'system' | 'light' | 'dark'

const STORAGE_KEY = 'mailx.theme'
const darkQuery = () => window.matchMedia('(prefers-color-scheme: dark)')

export function storedTheme(): ThemePreference {
  const value = localStorage.getItem(STORAGE_KEY)
  return value === 'light' || value === 'dark' ? value : 'system'
}

/** Applies the shadcn `.dark` class for a preference; `system` follows the OS. */
export function applyTheme(preference: ThemePreference) {
  const dark = preference === 'dark' || (preference === 'system' && darkQuery().matches)
  document.documentElement.classList.toggle('dark', dark)
}

export function useTheme() {
  const [theme, setTheme] = useState(storedTheme)

  useEffect(() => {
    applyTheme(theme)
    if (theme === 'system') localStorage.removeItem(STORAGE_KEY)
    else localStorage.setItem(STORAGE_KEY, theme)
    if (theme !== 'system') return
    const query = darkQuery()
    const follow = () => applyTheme('system')
    query.addEventListener('change', follow)
    return () => query.removeEventListener('change', follow)
  }, [theme])

  return [theme, setTheme] as const
}
