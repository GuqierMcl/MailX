import { QueryClientProvider } from '@tanstack/react-query'
import { AppearanceSync } from '@/features/settings/settings-queries'
import { ThemeToggle } from '@/features/settings/theme-toggle'
import { UpdaterProvider } from '@/features/updater/updater-provider'
import { useUpdater } from '@/features/updater/updater-context'
import { UpdateIndicator } from '@/features/updater/update-indicator'
import { Toaster } from '@/components/ui/toast'
import { TooltipProvider } from '@/components/ui/tooltip'
import { StartupGate } from '@/features/startup/startup-gate'
import { ComposerApp } from '@/features/composer/composer-app'
import { TitleBar, TitleBarSlotsProvider } from '@/features/window-chrome/title-bar'
import {
  createDesktopWindowHost,
  createPreviewWindowHost,
  previewPlatformFromUrl,
} from '@/features/window-chrome/window-host'
import { isTauri } from '@tauri-apps/api/core'
import { createDesktopClient, type MailxClient } from '@/lib/mailx-client'
import { windowRole } from '@/lib/composer-window'
import { createMockClient, seedPreviewOutbox } from '@/lib/mock-client'
import { createQueryClient } from '@/lib/query-client'
import { preloadMailBodyEditor } from '@/features/composer/mail-body-editor-loader'
import { SettingsSync } from '@/i18n/settings-sync'
import { createDesktopLifecycleClient, createPreviewLifecycleClient } from '@/features/lifecycle/lifecycle-client'
import { LifecycleProvider } from '@/features/lifecycle/lifecycle-provider'

// A plain browser in development gets synthetic data so the UI can be designed without the desktop host.
// `?composer=<id>` previews a composer window: that tab's own mock has no such draft, so it
// starts one (blank, first account). "Opening" a composer from the main preview opens such a tab.
const client = import.meta.env.DEV && !isTauri()
  ? createMockClient({
    seed: new URLSearchParams(window.location.search).get('seed'),
    resolveDraft: () => null,
    onOpenComposer: (draft, fresh) => {
      const params = new URLSearchParams(window.location.search)
      params.set('composer', draft.id)
      if (fresh) params.set('fresh', '1')
      else params.delete('fresh')
      window.open(`${window.location.pathname}?${params}`, `composer-${draft.id}`, 'popup,width=760,height=680')
    },
  })
  : createDesktopClient()

// Browser preview only: `?seed=outbox` fills the mock with a draft and with sends that end failed,
// unknown and waiting for a retry, so the drafts and outbox views can be looked at.
if (import.meta.env.DEV && !isTauri() && new URLSearchParams(window.location.search).get('seed') === 'outbox') {
  void seedPreviewOutbox(client)
}

const queryClient = createQueryClient()

const previewPlatform = import.meta.env.DEV ? previewPlatformFromUrl(window.location.search) : null
const role = windowRole()
const previewLifecycle = role.kind === 'main' && previewPlatform && !isTauri()
  ? createPreviewLifecycleClient(previewPlatform, client, previewPlatform === 'macos' || new URLSearchParams(window.location.search).get('tray') !== 'unavailable')
  : null
if (previewLifecycle && new URLSearchParams(window.location.search).get('close') === '1') void previewLifecycle.requestClose()
const lifecycle = previewLifecycle ?? createDesktopLifecycleClient()
const windowHost = previewPlatform ? createPreviewWindowHost(previewPlatform, () => { void previewLifecycle?.requestClose() }) : createDesktopWindowHost()
// Start the editor in this WebView before rendering/reading the draft. The main
// window still never imports the engine. Composer displays any load failure.
if (role.kind === 'composer') void preloadMailBodyEditor().catch(() => undefined)

function App() {
  if (role.kind === 'composer') {
    return (
      <QueryClientProvider client={queryClient}>
        <SettingsSync client={client} />
        <AppearanceSync client={client} importLegacy={false} />
        <Toaster />
        <TitleBarSlotsProvider>
          <TooltipProvider>
            <ComposerApp client={client} draftId={role.draftId} fresh={role.fresh} host={windowHost} />
          </TooltipProvider>
        </TitleBarSlotsProvider>
      </QueryClientProvider>
    )
  }
  return (
    <QueryClientProvider client={queryClient}>
      <SettingsSync client={client} />
      {/* Outside the startup gate: appearance applies even when the core failed to start. */}
      <AppearanceSync client={client} />
      {/* One global toast outlet; everything short that needs saying goes through `toast`. */}
      <Toaster />
      <LifecycleProvider client={lifecycle}>
        <TitleBarSlotsProvider>
          <div className="app-canvas flex h-svh flex-col overflow-hidden bg-sidebar text-foreground">
            <TitleBar host={windowHost} />
            {/* Encloses the gate too: Settings → About reads the update state. */}
            <UpdaterProvider client={client}>
              <UpdaterTitleBar client={client} />
              <div className="relative min-h-0 flex-1 overflow-auto">
                <StartupGate client={client} />
              </div>
            </UpdaterProvider>
          </div>
        </TitleBarSlotsProvider>
      </LifecycleProvider>
    </QueryClientProvider>
  )
}

/** The update button and the theme toggle share the trailing slot, in that order. */
function UpdaterTitleBar({ client }: { client: MailxClient }) {
  const { snapshot, openDialog } = useUpdater()
  return (
    <>
      <UpdateIndicator snapshot={snapshot} onOpen={openDialog} />
      <ThemeToggle client={client} />
    </>
  )
}

export default App
