import { useAppMenuCommands } from '@/features/app-menu/commands'
import { useLifecycle } from '@/features/lifecycle/lifecycle-context'
import { useUpdater } from '@/features/updater/updater-context'
import type { ThemeMode } from '@/generated/accounts'
import { i18n } from '@/i18n'
import type { MailxClient } from '@/lib/mailx-client'
import { requestSettings } from './settings-requests'
import { useSettings, useUpdateAppearance } from './settings-queries'

export function ApplicationMenuActions({ client }: { client: MailxClient }) {
  const lifecycle = useLifecycle()
  const settings = useSettings(client)
  const appearance = useUpdateAppearance(client)
  const theme = settings.data?.settings.appearance.theme
  const themeItem = (mode: ThemeMode) => ({
    enabled: theme !== undefined,
    checked: theme === mode,
    run: () => appearance.mutateAsync({ theme: mode }),
  })

  useAppMenuCommands({
    themeSystem: themeItem('system'),
    themeLight: themeItem('light'),
    themeDark: themeItem('dark'),
    quit: {
      enabled: Boolean(lifecycle) && !lifecycle?.snapshot?.quitting,
      run: () => lifecycle?.client.quit(),
    },
    guide: {
      run: () => {
        const path = i18n.language === 'zh-CN' ? 'zh-CN/README.md' : 'README.md'
        return client.openExternalLink(`https://github.com/GuqierMcl/MailX/blob/main/docs/guides/${path}`)
      },
    },
  })
  return null
}

/** These commands target the settings dialog owned by the ready workbench. */
export function PreferencesMenuActions() {
  const { snapshot, check } = useUpdater()
  useAppMenuCommands({
    settings: { run: () => requestSettings({ section: 'general' }) },
    signatures: { run: () => requestSettings({ section: 'signatures' }) },
    about: { run: () => requestSettings({ section: 'about' }) },
    shortcuts: { run: () => requestSettings({ section: 'shortcuts' }) },
    update: {
      enabled: snapshot.phase !== 'checking' && snapshot.phase !== 'downloading',
      run: () => {
        requestSettings({ section: 'about' })
        return check()
      },
    },
  })
  return null
}
