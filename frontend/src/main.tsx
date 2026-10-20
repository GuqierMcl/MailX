import { StrictMode } from 'react'
import { CSPProvider } from '@base-ui/react/csp-provider'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { applyAppearance, cachedAppearance } from './lib/appearance'

// Apply the last known appearance before the first paint to avoid a flash;
// the core's settings replace it once IPC answers.
applyAppearance(cachedAppearance())

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {/* The desktop CSP (style-src 'self') blocks the <style> tags Base UI would
        inject to hide native scrollbars; index.css carries those rules instead. */}
    <CSPProvider disableStyleElements>
      <App />
    </CSPProvider>
  </StrictMode>,
)
