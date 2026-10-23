import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, waitFor } from '@/test/render'
import { testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import { AppearanceSync } from './settings-queries'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'

afterEach(() => {
  localStorage.clear()
  document.documentElement.className = ''
  document.documentElement.removeAttribute('style')
})

describe('AppearanceSync', () => {
  it('applies the core settings and refreshes the first-paint cache', async () => {
    const client = testClient()
    const appearance = { ...DEFAULT_APPEARANCE, theme: 'dark' as const, accentColor: '#16a34a' }
    vi.mocked(client.getSettings).mockResolvedValue({ settings: { appearance, security: { remoteImages: 'ask', senderAvatars: false }, layout: { navWidth: 256 }, notifications: DEFAULT_NOTIFICATIONS }, stored: true, recovered: false })
    render(<AppearanceSync client={client} />)

    await waitFor(() => expect(document.documentElement).toHaveClass('dark'))
    expect(document.documentElement.style.getPropertyValue('--primary')).toBe('#16a34a')
    expect(JSON.parse(localStorage.getItem('mailx.appearance') ?? 'null')).toEqual(appearance)
  })

  it('imports the legacy theme once when no settings file exists yet', async () => {
    localStorage.setItem('mailx.theme', 'dark')
    const client = testClient()
    vi.mocked(client.getSettings).mockResolvedValue({ settings: { appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask', senderAvatars: false }, layout: { navWidth: 256 }, notifications: DEFAULT_NOTIFICATIONS }, stored: false, recovered: false })
    render(<AppearanceSync client={client} />)

    await waitFor(() => expect(localStorage.getItem('mailx.theme')).toBeNull())
    expect(client.updateSettings).toHaveBeenCalledOnce()
    expect(client.updateSettings).toHaveBeenCalledWith({ appearance: { ...DEFAULT_APPEARANCE, theme: 'dark' } })
    expect(document.documentElement).toHaveClass('dark')
  })

  it('leaves the legacy theme alone once settings are stored', async () => {
    localStorage.setItem('mailx.theme', 'dark')
    const client = testClient()
    render(<AppearanceSync client={client} />)

    await waitFor(() => expect(client.getSettings).toHaveBeenCalled())
    await waitFor(() => expect(localStorage.getItem('mailx.appearance')).not.toBeNull())
    expect(client.updateSettings).not.toHaveBeenCalled()
    expect(localStorage.getItem('mailx.theme')).toBe('dark')
  })
})
