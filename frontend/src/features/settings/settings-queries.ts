import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { AppearanceSettings, SettingsSnapshot } from '@/generated/accounts'
import { applyAppearance, cacheAppearance, clearLegacyTheme, legacyTheme } from '@/lib/appearance'
import type { MailxClient } from '@/lib/mailx-client'
import { settingsKeys } from '@/lib/query-client'

export function useSettings(client: MailxClient) {
  return useQuery({ queryKey: settingsKeys.all, queryFn: () => client.getSettings(), staleTime: Infinity })
}

/** Applies edits optimistically so the UI reacts at once; a rejected save rolls back. */
export function useUpdateAppearance(client: MailxClient) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (appearance: AppearanceSettings) => {
      const current = queryClient.getQueryData<SettingsSnapshot>(settingsKeys.all)
      return client.updateSettings({ ...current?.settings, appearance })
    },
    onMutate: async (appearance) => {
      await queryClient.cancelQueries({ queryKey: settingsKeys.all })
      const previous = queryClient.getQueryData<SettingsSnapshot>(settingsKeys.all)
      if (previous) {
        queryClient.setQueryData<SettingsSnapshot>(settingsKeys.all, {
          ...previous,
          settings: { ...previous.settings, appearance },
        })
      }
      return { previous }
    },
    onError: (_error, _appearance, context) => {
      if (context?.previous) queryClient.setQueryData(settingsKeys.all, context.previous)
    },
    onSuccess: (snapshot) => queryClient.setQueryData(settingsKeys.all, snapshot),
  })
}

/**
 * Keeps the document and the first-paint cache in step with the core's
 * settings, and imports the pre-`settings.json` theme preference once.
 */
export function AppearanceSync({ client }: { client: MailxClient }) {
  const { data } = useSettings(client)
  const { mutate } = useUpdateAppearance(client)
  const appearance = data?.settings.appearance

  useEffect(() => {
    if (!data || data.stored) return
    const theme = legacyTheme()
    if (theme) mutate({ ...data.settings.appearance, theme }, { onSuccess: clearLegacyTheme })
  }, [data, mutate])

  useEffect(() => {
    if (!appearance) return
    applyAppearance(appearance)
    cacheAppearance(appearance)
  }, [appearance])

  return null
}
