import { QueryClientProvider } from '@tanstack/react-query'
import { AppearanceSync } from '@/features/settings/settings-queries'
import { ThemeToggle } from '@/features/settings/theme-toggle'
import { Toaster } from '@/components/ui/toast'
import { StartupGate } from '@/features/startup/startup-gate'
import { TitleBar, TitleBarSlotsProvider } from '@/features/window-chrome/title-bar'
import {
  createDesktopWindowHost,
  createPreviewWindowHost,
  previewPlatformFromUrl,
} from '@/features/window-chrome/window-host'
import { isTauri } from '@tauri-apps/api/core'
import { createDesktopClient } from '@/lib/mailx-client'
import { createMockClient } from '@/lib/mock-client'
import { createQueryClient } from '@/lib/query-client'

// A plain browser in development gets synthetic data so the UI can be designed without the desktop host.
const client = import.meta.env.DEV && !isTauri() ? createMockClient() : createDesktopClient()

const queryClient = createQueryClient()

const previewPlatform = import.meta.env.DEV ? previewPlatformFromUrl(window.location.search) : null
const windowHost = previewPlatform ? createPreviewWindowHost(previewPlatform) : createDesktopWindowHost()

function App() {
  return (
    <QueryClientProvider client={queryClient}>
      {/* Outside the startup gate: appearance applies even when the core failed to start. */}
      <AppearanceSync client={client} />
      {/* One global toast outlet; everything short that needs saying goes through `toast`. */}
      <Toaster />
      <TitleBarSlotsProvider>
        <div className="app-canvas flex h-svh flex-col overflow-hidden bg-sidebar text-foreground">
          <TitleBar host={windowHost} />
          <ThemeToggle client={client} />
          <div className="relative min-h-0 flex-1 overflow-auto">
            <StartupGate client={client} />
          </div>
        </div>
      </TitleBarSlotsProvider>
    </QueryClientProvider>
  )
}

export default App
