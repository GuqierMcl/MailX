import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from './App'

// File isolation gives this window its own real App module and clients, without
// resetting and importing the entire module graph inside a timed test.
vi.hoisted(() => {
  window.history.replaceState(null, '', '/?composer=00000000-0000-4000-8000-000000000000')
})

afterEach(() => { window.history.replaceState(null, '', '/') })

describe('the real window trees', () => {
  it('renders no updater UI in a composer window', () => {
    render(<App />)
    expect(screen.queryByRole('button', { name: /检查更新|Check for updates/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /MailX .+ (可用|is available)/ })).not.toBeInTheDocument()
    // The provider is not mounted there, so there is no update state at all.
    expect(document.querySelector('[data-slot="title-bar-trailing"] button[aria-label*="MailX "]')).toBeNull()
  })
})
