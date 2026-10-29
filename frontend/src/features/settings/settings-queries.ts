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
import { registerPendingLayoutSave } from './pending-layout-saves'

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
export const useUpdateGeneral = (client: MailxClient) => useSettingsSection(client, 'general', { language: 'system', autoCheckUpdates: true, closeBehavior: 'ask', showMenuBarIcon: false }, false)

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
  const { mutateAsync } = useUpdateLayout(client)
  const queryClient = useQueryClient()
  const pending = useRef<Partial<LayoutSettings>>({})
  const inFlight = useRef<Promise<void> | null>(null)
  const flush = useCallback((): Promise<void> => {
    if (inFlight.current) return inFlight.current
    const saving = (async () => {
      while (Object.keys(pending.current).length > 0) {
        const change = pending.current
        pending.current = {}
        try { await mutateAsync(change) } catch (cause) {
          // Newer edits win, but a failed value stays available for a later retry.
          pending.current = { ...change, ...pending.current }
          throw cause
        }
      }
    })()
    inFlight.current = saving
    const settled = () => { if (inFlight.current === saving) inFlight.current = null }
    void saving.then(settled, settled)
    return saving
  }, [mutateAsync])
  // Timer / pagehide callbacks have no caller to receive a rejection. The
  // pending values remain in the registry, where explicit quit can retry them.
  const write = useCallback(() => { void flush().catch(() => undefined) }, [flush])
  const debounced = useDebouncedCallback(write, { wait: LAYOUT_SAVE_DELAY_MS })
  useEffect(() => {
    let mounted = true
    const registeredFlush = async () => {
      await flush()
      if (!mounted) unregister()
    }
    const unregister = registerPendingLayoutSave(queryClient, registeredFlush)
    return () => {
      mounted = false
      // Keep a failed unmounted writer registered until its values are saved.
      void registeredFlush().catch(() => undefined)
    }
  }, [flush, queryClient])
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
