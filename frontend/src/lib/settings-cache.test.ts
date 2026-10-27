import { describe, expect, it } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { createQueryClient, settingsKeys } from './query-client'
import { acceptSettingsSnapshot, beginSettingsEdit, committedSettings, finishSettingsEdit, latestSettingsSnapshot } from './settings-cache'
import type { SettingsSnapshot } from '@/generated/accounts'

describe('settings snapshots and pending edits', () => {
  it('rejects a late response after a newer language was committed', async () => {
    const cache = createQueryClient()
    const old = await testClient().getSettings()
    const latest: SettingsSnapshot = { ...old, revision: 2, effectiveLocale: 'en', settings: { ...old.settings, general: { language: 'en', autoCheckUpdates: true } } }
    acceptSettingsSnapshot(cache, latest)
    acceptSettingsSnapshot(cache, old)
    expect(cache.getQueryData(settingsKeys.all)).toEqual(latest)
  })

  it('rolls back only the failed edit while retaining a newer language and another pending edit', async () => {
    const cache = createQueryClient()
    const initial = await testClient().getSettings()
    acceptSettingsSnapshot(cache, initial)
    const appearance = beginSettingsEdit(cache, 'appearance', { theme: 'dark' })
    const layout = beginSettingsEdit(cache, 'layout', { navWidth: 300 })
    acceptSettingsSnapshot(cache, { ...initial, revision: 2, effectiveLocale: 'en', settings: { ...initial.settings, general: { language: 'en', autoCheckUpdates: true } } })
    finishSettingsEdit(cache, appearance)
    const result = cache.getQueryData<SettingsSnapshot>(settingsKeys.all)!
    expect(result.settings.general.language).toBe('en')
    expect(result.settings.appearance.theme).toBe(initial.settings.appearance.theme)
    expect(result.settings.layout.navWidth).toBe(300)
    finishSettingsEdit(cache, layout)
    expect(cache.getQueryData<SettingsSnapshot>(settingsKeys.all)!.settings.layout.navWidth).toBe(initial.settings.layout.navWidth)
  })

  it('keeps queued UI edits out of the base used for saving another change', async () => {
    const cache = createQueryClient()
    const initial = await testClient().getSettings()
    acceptSettingsSnapshot(cache, initial)
    beginSettingsEdit(cache, 'appearance', { accentColor: '#2563eb' })
    beginSettingsEdit(cache, 'appearance', { interfaceFont: 'Pending font' })
    expect(committedSettings(cache)?.appearance).toEqual(initial.settings.appearance)
    expect(cache.getQueryData<SettingsSnapshot>(settingsKeys.all)!.settings.appearance.interfaceFont).toBe('Pending font')
  })

  it('rechecks a snapshot at the final query write when an event arrives in the intervening microtask', async () => {
    const cache = createQueryClient()
    const old = await testClient().getSettings()
    const latest: SettingsSnapshot = { ...old, revision: 2, effectiveLocale: 'en', settings: { ...old.settings, general: { language: 'en', autoCheckUpdates: true } } }
    await cache.fetchQuery({
      queryKey: settingsKeys.all,
      queryFn: async () => {
        const result = acceptSettingsSnapshot(cache, old)
        queueMicrotask(() => acceptSettingsSnapshot(cache, latest))
        return result
      },
      structuralSharing: (_previous, incoming) => latestSettingsSnapshot(cache) ?? incoming,
    })
    expect(cache.getQueryData<SettingsSnapshot>(settingsKeys.all)?.effectiveLocale).toBe('en')
  })
})
