import { useEffect, useRef } from 'react'
import { toast } from '@/components/ui/toast'
import type { WindowHost } from '@/features/window-chrome/window-host'
import { t } from '@/i18n'
import type { MailxClient } from '@/lib/mailx-client'
import { releaseUrl } from './details'
import { useUpdater } from './updater-context'

const NOTICE_DURATION_MS = 8000

/** Mounted once the workbench is ready; waits for a silent launch to come forward. */
export function InstalledUpdateNotice({ client, host }: { client: MailxClient; host: WindowHost | null }) {
  const { snapshot: { installedUpdateVersion: version } } = useUpdater()
  const shown = useRef<string | null>(null)

  useEffect(() => {
    if (!version || !host) return
    let disposed = false
    let observed = false
    let acknowledging = false
    const present = (focused: boolean) => {
      if (disposed || !focused || acknowledging) return
      if (shown.current !== version) {
        shown.current = version
        toast.success(t('updater:installedToast', { version }), {
          timeout: NOTICE_DURATION_MS,
          action: {
            label: t('updater:releaseNotes'),
            onClick: () => void client.openExternalLink(releaseUrl(version)).catch(() => {
              toast.error(t('updater:releaseNotesOpenFailed'))
            }),
          },
        })
      }
      acknowledging = true
      void client.acknowledgeUpdateInstalled(version).catch(() => { acknowledging = false })
    }
    const stop = host.onStateChange((state) => {
      observed = true
      present(state.focused)
    })
    void host.getChrome().then(({ state }) => {
      if (!observed) present(state.focused)
    }, () => undefined)
    return () => { disposed = true; stop() }
  }, [client, host, version])

  return null
}
