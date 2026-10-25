import { StrictMode } from 'react'
import { CSPProvider } from '@base-ui/react/csp-provider'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { isTauri } from '@tauri-apps/api/core'
import { applyAppearance, cachedAppearance } from './lib/appearance'
import { installBrowserDefaultGuards } from './lib/browser-defaults'
import { initializeLanguage } from './i18n/locale'

// Apply the last known appearance before the first paint to avoid a flash;
// the core's settings replace it once IPC answers.
applyAppearance(cachedAppearance())
initializeLanguage()

// A packaged app should not act like a browser tab. Development builds keep
// reload and the inspector for working on the UI.
if (import.meta.env.PROD && isTauri()) installBrowserDefaultGuards()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {/* The desktop CSP (style-src 'self') blocks the <style> tags Base UI would
        inject to hide native scrollbars; index.css carries those rules instead. */}
    <CSPProvider disableStyleElements>
      <App />
    </CSPProvider>
  </StrictMode>,
)
