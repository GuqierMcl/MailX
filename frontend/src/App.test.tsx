import { describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'

// App chooses its window role and clients when the module is evaluated. Load
// the real tree before the test timer starts: a cold Vite module transform is
// test setup, not time spent rendering or interacting with the application.
vi.hoisted(() => { window.history.replaceState(null, '', '/') })

// The browser preview switches to the system language, so names may be either.
const matcher = (zh: string, en: string) => new RegExp(`${zh}|${en}`)

describe('the real window trees', () => {
  it('renders Settings → About in the main window with no test-supplied provider', async () => {
    // A regression guard: the updater provider must enclose the startup gate,
    // or `useUpdater` in the About section throws as soon as Settings opens.
    render(<App />)
    await userEvent.click(await screen.findByRole('button', { name: matcher('设置', 'Settings') }, { timeout: 5000 }))
    // Toasts also have a dialog role; wait for the named settings dialog.
    const dialog = await screen.findByRole('dialog', { name: matcher('设置', 'Settings') })
    await userEvent.click(within(dialog).getByRole('button', { name: matcher('关于', 'About') }))
    expect(await within(dialog).findByRole('button', { name: matcher('检查更新', 'Check for updates') })).toBeInTheDocument()
    expect(within(dialog).getByRole('switch', { name: matcher('自动更新', 'Automatic updates') })).toBeInTheDocument()
  })
})
