import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, render, waitFor } from '@/test/render'
import { testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import { AppearanceSync, LAYOUT_SAVE_DELAY_MS, useDebouncedLayoutSave } from './settings-queries'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'

afterEach(() => {
  localStorage.clear()
  document.documentElement.className = ''
  document.documentElement.removeAttribute('style')
})

describe('AppearanceSync', () => {
  it('applies the core settings and refreshes the first-paint cache', async () => {
    const client = testClient()
    const appearance = { ...DEFAULT_APPEARANCE, theme: 'dark' as const, accentColor: '#16a34a' }
    vi.mocked(client.getSettings).mockResolvedValue({ effectiveLocale: 'zh-CN', revision: 0, settings: { general: { language: 'zh-CN', autoCheckUpdates: true, closeBehavior: 'ask', showMenuBarIcon: false }, appearance, security: { remoteImages: 'ask', senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, compose: { defaultAccountId: '', undoSendSeconds: 0 } }, stored: true, recovered: false })
    render(<AppearanceSync client={client} />)

    await waitFor(() => expect(document.documentElement).toHaveClass('dark'))
    expect(document.documentElement.style.getPropertyValue('--primary')).toBe('#16a34a')
    expect(JSON.parse(localStorage.getItem('mailx.appearance') ?? 'null')).toEqual(appearance)
  })

  it('imports the legacy theme once when no settings file exists yet', async () => {
    localStorage.setItem('mailx.theme', 'dark')
    const client = testClient()
    vi.mocked(client.getSettings).mockResolvedValue({ effectiveLocale: 'zh-CN', revision: 0, settings: { general: { language: 'zh-CN', autoCheckUpdates: true, closeBehavior: 'ask', showMenuBarIcon: false }, appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask', senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, compose: { defaultAccountId: '', undoSendSeconds: 0 } }, stored: false, recovered: false })
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


describe('useDebouncedLayoutSave', () => {
  function Saver({ client, changes }: { client: ReturnType<typeof testClient>; changes: { current: ((change: Partial<typeof DEFAULT_LAYOUT>) => void) | null } }) {
    changes.current = useDebouncedLayoutSave(client)
    return null
  }
  const setup = () => {
    vi.useFakeTimers()
    const client = testClient()
    const changes: { current: ((change: Partial<typeof DEFAULT_LAYOUT>) => void) | null } = { current: null }
    const view = render(<Saver client={client} changes={changes} />)
    const save = (change: Partial<typeof DEFAULT_LAYOUT>) => act(() => changes.current!(change))
    const wait = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms) })
    return { client, view, save, wait }
  }
  afterEach(() => vi.useRealTimers())

  it('turns a burst of changes into one write of the last values', async () => {
    const { client, save, wait } = setup()
    for (const width of [260, 270, 280, 290, 300]) {
      save({ navWidth: width })
      await wait(50)
    }
    expect(client.updateSettings).not.toHaveBeenCalled()
    await wait(LAYOUT_SAVE_DELAY_MS)
    expect(client.updateSettings).toHaveBeenCalledOnce()
    expect(client.updateSettings).toHaveBeenCalledWith({ layout: { ...DEFAULT_LAYOUT, navWidth: 300 } })
  })

  it('merges changes to different parts of the layout instead of losing one', async () => {
    const { client, save, wait } = setup()
    save({ navWidth: 300 })
    await wait(100)
    save({ listWidth: 500 })
    await wait(100)
    save({ readerCollapsed: true })
    await wait(LAYOUT_SAVE_DELAY_MS)
    expect(client.updateSettings).toHaveBeenCalledOnce()
    expect(client.updateSettings).toHaveBeenCalledWith({ layout: { navWidth: 300, listWidth: 500, readerCollapsed: true } })
  })

  it('writes again for changes made after a write, and never writes nothing', async () => {
    const { client, save, wait } = setup()
    await wait(LAYOUT_SAVE_DELAY_MS * 3)
    expect(client.updateSettings).not.toHaveBeenCalled()
    save({ navWidth: 300 })
    await wait(LAYOUT_SAVE_DELAY_MS)
    save({ navWidth: 320 })
    await wait(LAYOUT_SAVE_DELAY_MS)
    expect(client.updateSettings).toHaveBeenCalledTimes(2)
    expect(client.updateSettings).toHaveBeenLastCalledWith({ layout: { ...DEFAULT_LAYOUT, navWidth: 320 } })
  })

  it('writes what is waiting when the page is closed or the view goes away', async () => {
    const closing = setup()
    closing.save({ listWidth: 600 })
    act(() => { window.dispatchEvent(new Event('pagehide')) })
    // The mutation starts on the next tick; nothing waits for the debounce delay.
    await closing.wait(0)
    expect(closing.client.updateSettings).toHaveBeenCalledOnce()
    expect(closing.client.updateSettings).toHaveBeenCalledWith({ layout: { ...DEFAULT_LAYOUT, listWidth: 600 } })
    await closing.wait(LAYOUT_SAVE_DELAY_MS * 2)
    expect(closing.client.updateSettings).toHaveBeenCalledOnce()
    closing.view.unmount()
    vi.useRealTimers()

    const leaving = setup()
    leaving.save({ navWidth: 310 })
    leaving.view.unmount()
    await leaving.wait(0)
    expect(leaving.client.updateSettings).toHaveBeenCalledOnce()
    expect(leaving.client.updateSettings).toHaveBeenCalledWith({ layout: { ...DEFAULT_LAYOUT, navWidth: 310 } })
  })
})
