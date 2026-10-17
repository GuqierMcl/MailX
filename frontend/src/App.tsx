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

// A plain browser in development gets synthetic data so the UI can be designed without the desktop host.
const client = import.meta.env.DEV && !isTauri() ? createMockClient() : createDesktopClient()

const previewPlatform = import.meta.env.DEV ? previewPlatformFromUrl(window.location.search) : null
const windowHost = previewPlatform ? createPreviewWindowHost(previewPlatform) : createDesktopWindowHost()

function App() {
  return (
    <TitleBarSlotsProvider>
      <div className="flex h-svh flex-col overflow-hidden">
        <TitleBar host={windowHost} title="MailX" />
        <div className="relative min-h-0 flex-1 overflow-auto">
          <StartupGate client={client} />
        </div>
      </div>
    </TitleBarSlotsProvider>
  )
}

export default App
