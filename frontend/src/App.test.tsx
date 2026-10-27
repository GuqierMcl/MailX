import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@/test/render'
import userEvent from '@testing-library/user-event'

// The browser preview switches to the system language, so names may be either.
const matcher = (zh: string, en: string) => new RegExp(`${zh}|${en}`)

afterEach(() => {
  window.history.replaceState(null, '', '/')
  vi.resetModules()
})

async function loadApp(search: string) {
  window.history.replaceState(null, '', `/${search}`)
  vi.resetModules()
  return (await import('./App')).default
}

/** The settings dialog; a toast is also a dialog, so pick the one with a title. */
async function settingsDialog() {
  const dialogs = await screen.findAllByRole('dialog')
  const dialog = dialogs.find((element) => element.querySelector('[data-slot="dialog-title"]'))
  if (!dialog) throw new Error('settings dialog did not open')
  return dialog
}

describe('the real window trees', () => {
  it('renders Settings → About in the main window with no test-supplied provider', async () => {
    // A regression guard: the updater provider must enclose the startup gate,
    // or `useUpdater` in the About section throws as soon as Settings opens.
    const App = await loadApp('')
    render(<App />)
    await userEvent.click(await screen.findByRole('button', { name: matcher('设置', 'Settings') }, { timeout: 5000 }))
    const dialog = await settingsDialog()
    await userEvent.click(within(dialog).getByRole('button', { name: matcher('关于', 'About') }))
    expect(await within(dialog).findByRole('button', { name: matcher('检查更新', 'Check for updates') })).toBeInTheDocument()
    expect(within(dialog).getByRole('switch', { name: matcher('自动检查更新', 'Check for updates automatically') })).toBeInTheDocument()
  })

  it('renders no updater UI in a composer window', async () => {
    const App = await loadApp('?composer=00000000-0000-4000-8000-000000000000')
    render(<App />)
    expect(screen.queryByRole('button', { name: matcher('检查更新', 'Check for updates') })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /MailX .+ (可用|is available)/ })).not.toBeInTheDocument()
    // The provider is not mounted there, so there is no update state at all.
    expect(document.querySelector('[data-slot="title-bar-trailing"] button[aria-label*="MailX "]')).toBeNull()
  })
})
