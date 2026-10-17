import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { applyTheme, storedTheme } from './lib/theme'

// Apply the saved theme before the first paint to avoid a light flash.
applyTheme(storedTheme())

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
