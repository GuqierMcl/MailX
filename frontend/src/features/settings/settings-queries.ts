import { useCallback, useEffect, useRef } from 'react'
import { useDebouncedCallback } from '@tanstack/react-pacer'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { AppSettings, LayoutSettings } from '@/generated/accounts'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'
import { DEFAULT_APPEARANCE, applyAppearance, cacheAppearance, clearLegacyTheme, legacyTheme } from '@/lib/appearance'
import type { MailxClient } from '@/lib/mailx-client'
import { settingsKeys } from '@/lib/query-client'
import { acceptSettingsSnapshot, beginSettingsEdit, committedSettings, finishSettingsEdit, latestSettingsSnapshot } from '@/lib/settings-cache'
import { ensureSettingsWatch, retainSettingsWatch } from '@/i18n/settings-watch'

export function useSettings(client: MailxClient) {
  const queryClient = useQueryClient()
  useEffect(() => retainSettingsWatch(client, queryClient), [client, queryClient])
  return useQuery({
    queryKey: settingsKeys.all,
    queryFn: async () => {
      await ensureSettingsWatch(client, queryClient)
      return acceptSettingsSnapshot(queryClient, await client.getSettings())
    },
    staleTime: Infinity,
    structuralSharing: (_previous, incoming) => latestSettingsSnapshot(queryClient) ?? incoming,
  })
}

/** Canonical snapshots and pending edits stay separate, including across host events. */
function useSettingsSection<K extends keyof AppSettings>(client: MailxClient, section: K, defaults: AppSettings[K], optimistic = true) {
  const queryClient = useQueryClient()
  return useMutation({
    scope: { id: `settings-${section}` },
    mutationFn: (change: Partial<AppSettings[K]>) => {
      const current = committedSettings(queryClient)?.[section]
      return client.updateSettings({ [section]: { ...defaults, ...current, ...change } } as Partial<AppSettings>)
    },
    onMutate: async (change) => {
      await queryClient.cancelQueries({ queryKey: settingsKeys.all })
      return optimistic ? beginSettingsEdit(queryClient, section, change) : undefined
    },
    onSuccess: (snapshot, _change, token) => finishSettingsEdit(queryClient, token, snapshot),
    onError: (_error, _change, token) => finishSettingsEdit(queryClient, token),
  })
}

export const useUpdateAppearance = (client: MailxClient) => useSettingsSection(client, 'appearance', DEFAULT_APPEARANCE)
export const useUpdateSecurity = (client: MailxClient) => useSettingsSection(client, 'security', { remoteImages: 'ask', senderAvatars: false })
export const useUpdateLayout = (client: MailxClient) => useSettingsSection(client, 'layout', DEFAULT_LAYOUT)
export const useUpdateNotifications = (client: MailxClient) => useSettingsSection(client, 'notifications', DEFAULT_NOTIFICATIONS)
export const useUpdateCompose = (client: MailxClient) => useSettingsSection(client, 'compose', { defaultAccountId: '', undoSendSeconds: 0 })
export const useUpdateGeneral = (client: MailxClient) => useSettingsSection(client, 'general', { language: 'system', autoCheckUpdates: true }, false)

/** How long layout changes wait for a quieter moment before being written. */
export const LAYOUT_SAVE_DELAY_MS = 500

/**
 * Layout changes (a dragged width, a folded reader) written to disk once the
 * user pauses, not on every event: a burst of key presses or a few drags in a
 * row becomes one write. Changes made in the meantime are merged, so two
 * writers (sidebar, list) cannot lose each other's value. What is still waiting
 * is written when the page goes away or the caller unmounts.
 */
export function useDebouncedLayoutSave(client: MailxClient) {
  const { mutate } = useUpdateLayout(client)
  const pending = useRef<Partial<LayoutSettings>>({})
  const write = useCallback(() => {
    const change = pending.current
    if (Object.keys(change).length === 0) return
    pending.current = {}
    mutate(change)
  }, [mutate])
  const debounced = useDebouncedCallback(write, { wait: LAYOUT_SAVE_DELAY_MS, onUnmount: (debouncer) => debouncer.flush() })
  useEffect(() => {
    // The window can close inside the delay; `pagehide` is the last chance to send it.
    window.addEventListener('pagehide', write)
    return () => window.removeEventListener('pagehide', write)
  }, [write])
  return useCallback((change: Partial<LayoutSettings>) => {
    pending.current = { ...pending.current, ...change }
    debounced()
  }, [debounced])
}

/**
 * Keeps the document and the first-paint cache in step with the core's
 * settings, and imports the pre-`settings.json` theme preference once.
 */
export function AppearanceSync({ client, importLegacy = true }: { client: MailxClient; importLegacy?: boolean }) {
  const { data } = useSettings(client)
  const { mutate } = useUpdateAppearance(client)
  const appearance = data?.settings.appearance

  useEffect(() => {
    // Only the main window may write settings; a composer window just follows them.
    if (!importLegacy || !data || data.stored) return
    const theme = legacyTheme()
    if (theme) mutate({ ...data.settings.appearance, theme }, { onSuccess: clearLegacyTheme })
  }, [data, mutate, importLegacy])

  useEffect(() => {
    if (!appearance) return
    applyAppearance(appearance)
    cacheAppearance(appearance)
  }, [appearance])

  return null
}
