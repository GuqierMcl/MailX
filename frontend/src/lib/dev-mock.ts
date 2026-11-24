import { isTauri } from '@tauri-apps/api/core'

/**
 * The mock backend in a development build. A plain browser always gets it; the desktop dev app
 * (`bun run desktop:dev:isolated`) gets it only when `VITE_MAILX_SEED` is set, because its window
 * cannot carry a URL parameter. Production builds never read the variable, and `DEV` is false there.
 */
export const mockBackend = import.meta.env.DEV && (!isTauri() || Boolean(import.meta.env.VITE_MAILX_SEED))

/** Which synthetic dataset: `?seed=` in a browser, `VITE_MAILX_SEED` in the desktop dev app. */
export function mockSeed(search = window.location.search): string | null {
  return isTauri() ? import.meta.env.VITE_MAILX_SEED || null : new URLSearchParams(search).get('seed')
}
