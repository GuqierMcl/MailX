import { useSyncExternalStore } from 'react'
import { useSettings } from '@/features/settings/settings-queries'
import type { MailxClient } from '@/lib/mailx-client'

function subscribeVisibility(listener: () => void) {
  document.addEventListener('visibilitychange', listener)
  return () => document.removeEventListener('visibilitychange', listener)
}

function subscribeAppTheme(listener: () => void) {
  const observer = new MutationObserver(listener)
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
  return () => observer.disconnect()
}

export function useBodyEnvironment(client: MailxClient) {
  const visible = useSyncExternalStore(subscribeVisibility, () => document.visibilityState !== 'hidden')
  const settings = useSettings(client)
  const appDark = useSyncExternalStore(subscribeAppTheme, () => document.documentElement.classList.contains('dark'))
  const dark = appDark && (settings.data?.settings.appearance.mailTheme ?? 'follow') === 'follow'
  return { visible, settings, dark }
}
