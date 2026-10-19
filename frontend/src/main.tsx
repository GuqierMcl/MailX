import { StrictMode } from 'react'
import { CSPProvider } from '@base-ui/react/csp-provider'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { applyTheme, storedTheme } from './lib/theme'

// Apply the saved theme before the first paint to avoid a light flash.
applyTheme(storedTheme())

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {/* The desktop CSP (style-src 'self') blocks the <style> tags Base UI would
        inject to hide native scrollbars; index.css carries those rules instead. */}
    <CSPProvider disableStyleElements>
      <App />
    </CSPProvider>
  </StrictMode>,
)
