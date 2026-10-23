import { useCallback, useEffect, useRef } from 'react'
import { useDebouncedCallback } from '@tanstack/react-pacer'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { AppearanceSettings, LayoutSettings, NotificationSettings, SecuritySettings, SettingsSnapshot } from '@/generated/accounts'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'
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
    scope: { id: 'appearance-settings' },
    mutationFn: (appearance: AppearanceSettings) => {
      return client.updateSettings({ appearance })
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
      if (context?.previous) queryClient.setQueryData<SettingsSnapshot>(settingsKeys.all, (current) => current && ({
        ...current, settings: { ...current.settings, appearance: context.previous!.settings.appearance },
      }))
    },
    onSuccess: (snapshot) => queryClient.setQueryData<SettingsSnapshot>(settingsKeys.all, (current) => ({
      ...snapshot, settings: {
        ...snapshot.settings,
        security: current?.settings.security ?? snapshot.settings.security,
        layout: current?.settings.layout ?? snapshot.settings.layout,
        notifications: current?.settings.notifications ?? snapshot.settings.notifications,
      },
    })),
  })
}

export function useUpdateSecurity(client: MailxClient) {
  const queryClient = useQueryClient()
  return useMutation({
    scope: { id: 'security-settings' },
    // Sections are replaced whole, so merge the change into the current values.
    mutationFn: (change: Partial<SecuritySettings>) => {
      const current = queryClient.getQueryData<SettingsSnapshot>(settingsKeys.all)?.settings.security
      return client.updateSettings({ security: { remoteImages: 'ask', senderAvatars: false, ...current, ...change } })
    },
    onSuccess: (snapshot) => queryClient.setQueryData<SettingsSnapshot>(settingsKeys.all, (current) => ({
      ...snapshot, settings: {
        ...snapshot.settings,
        appearance: current?.settings.appearance ?? snapshot.settings.appearance,
        layout: current?.settings.layout ?? snapshot.settings.layout,
        notifications: current?.settings.notifications ?? snapshot.settings.notifications,
      },
    })),
  })
}

/**
 * Saves window layout such as the sidebar and list widths; the caller shows its
 * own value meanwhile. A change names only what moved: the rest of the layout
 * is taken from the saved copy when the save starts, so two writers (sidebar,
 * list) never overwrite each other's width.
 */
export function useUpdateLayout(client: MailxClient) {
  const queryClient = useQueryClient()
  return useMutation({
    scope: { id: 'layout-settings' },
    mutationFn: (change: Partial<LayoutSettings>) => {
      const current = queryClient.getQueryData<SettingsSnapshot>(settingsKeys.all)?.settings.layout
      return client.updateSettings({ layout: { ...DEFAULT_LAYOUT, ...current, ...change } })
    },
    onSuccess: (snapshot) => queryClient.setQueryData<SettingsSnapshot>(settingsKeys.all, (current) => ({
      ...snapshot, settings: {
        ...snapshot.settings,
        appearance: current?.settings.appearance ?? snapshot.settings.appearance,
        security: current?.settings.security ?? snapshot.settings.security,
        notifications: current?.settings.notifications ?? snapshot.settings.notifications,
      },
    })),
  })
}

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

/**
 * Saves notification preferences. Applied optimistically so switches respond
 * at once; a rejected save puts the previous values back.
 */
export function useUpdateNotifications(client: MailxClient) {
  const queryClient = useQueryClient()
  return useMutation({
    scope: { id: 'notification-settings' },
    mutationFn: (change: Partial<NotificationSettings>) => {
      const current = queryClient.getQueryData<SettingsSnapshot>(settingsKeys.all)?.settings.notifications
      return client.updateSettings({ notifications: { ...DEFAULT_NOTIFICATIONS, ...current, ...change } })
    },
    onMutate: async (change) => {
      await queryClient.cancelQueries({ queryKey: settingsKeys.all })
      const previous = queryClient.getQueryData<SettingsSnapshot>(settingsKeys.all)
      if (previous) {
        queryClient.setQueryData<SettingsSnapshot>(settingsKeys.all, {
          ...previous,
          settings: { ...previous.settings, notifications: { ...previous.settings.notifications, ...change } },
        })
      }
      return { previous }
    },
    onError: (_error, _change, context) => {
      if (context?.previous) queryClient.setQueryData<SettingsSnapshot>(settingsKeys.all, (current) => current && ({
        ...current, settings: { ...current.settings, notifications: context.previous!.settings.notifications },
      }))
    },
    onSuccess: (snapshot) => queryClient.setQueryData<SettingsSnapshot>(settingsKeys.all, (current) => ({
      ...snapshot, settings: {
        ...snapshot.settings,
        appearance: current?.settings.appearance ?? snapshot.settings.appearance,
        security: current?.settings.security ?? snapshot.settings.security,
        layout: current?.settings.layout ?? snapshot.settings.layout,
      },
    })),
  })
}
