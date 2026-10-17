import { StartupGate } from '@/features/startup/startup-gate'
import { TitleBar } from '@/features/window-chrome/title-bar'
import {
  createDesktopWindowHost,
  createPreviewWindowHost,
  previewPlatformFromUrl,
} from '@/features/window-chrome/window-host'
import { createDesktopClient } from '@/lib/mailx-client'
import './App.css'

const client = createDesktopClient()

const previewPlatform = import.meta.env.DEV ? previewPlatformFromUrl(window.location.search) : null
const windowHost = previewPlatform ? createPreviewWindowHost(previewPlatform) : createDesktopWindowHost()

function App() {
  return (
    <div className="flex h-svh flex-col overflow-hidden">
      <TitleBar host={windowHost} title="MailX" />
      <div className="relative min-h-0 flex-1 overflow-auto">
        <StartupGate client={client} />
      </div>
    </div>
  )
}

export default App
