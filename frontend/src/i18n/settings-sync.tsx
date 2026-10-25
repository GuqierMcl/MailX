import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { MailxClient } from '@/lib/mailx-client'
import { acceptSettingsSnapshot } from '@/lib/settings-cache'
import { applyLanguage } from './locale'
import { ensureSettingsWatch, retainSettingsWatch } from './settings-watch'
import { useSettings } from '@/features/settings/settings-queries'

export function SettingsSync({ client }: { client: MailxClient }) {
  const queryClient = useQueryClient()
  const { data } = useSettings(client)
  useEffect(() => {
    const refresh = () => {
      void ensureSettingsWatch(client, queryClient).then(() => client.getSettings())
        .then((snapshot) => acceptSettingsSnapshot(queryClient, snapshot)).catch(() => undefined)
    }
    const release = retainSettingsWatch(client, queryClient)
    window.addEventListener('focus', refresh)
    return () => {
      window.removeEventListener('focus', refresh)
      release()
    }
  }, [client, queryClient])
  useEffect(() => {
    if (data) applyLanguage(data.effectiveLocale, data.settings.general.language)
  }, [data])
  return null
}
