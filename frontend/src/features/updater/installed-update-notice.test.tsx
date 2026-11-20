import { StrictMode } from 'react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { Toaster } from '@/components/ui/toast'
import { testClient } from '@/features/accounts/test-fixtures'
import type { WindowChrome, WindowHost, WindowState } from '@/features/window-chrome/window-host'
import { act, render, screen, waitFor, within } from '@/test/render'
import { InstalledUpdateNotice } from './installed-update-notice'
import type { UpdateSnapshot } from './types'
import { UpdaterProvider } from './updater-provider'

const installed = { canInstall: true, phase: 'idle', installedUpdateVersion: '0.2.0' } satisfies UpdateSnapshot
const title = 'MailX 已更新至 0.2.0'

function chrome(focused: boolean): WindowChrome {
  return { platform: 'macos', leading: [], trailing: [], translucent: false, state: { focused, maximized: false, fullscreen: false } }
}

function setup(snapshot: UpdateSnapshot, initialChrome: Promise<WindowChrome>) {
  const client = testClient()
  client.getUpdateState.mockResolvedValue(snapshot)
  let emitUpdate: (next: UpdateSnapshot) => void = () => undefined
  client.onUpdateState.mockImplementation((listener) => { emitUpdate = listener; return () => undefined })
  const listeners = new Set<(state: WindowState) => void>()
  const host = {
    getChrome: vi.fn().mockReturnValue(initialChrome),
    onStateChange(listener) { listeners.add(listener); return () => { listeners.delete(listener) } },
    perform: vi.fn(),
  } satisfies WindowHost
  render(<StrictMode><Toaster /><UpdaterProvider client={client}><InstalledUpdateNotice client={client} host={host} /></UpdaterProvider></StrictMode>)
  return {
    client, host,
    focus: (focused: boolean) => act(() => listeners.forEach((listener) => listener(chrome(focused).state))),
    update: (next: UpdateSnapshot) => act(() => emitUpdate(next)),
  }
}

describe('installed update notice', () => {
  it('shows once, acknowledges presentation, and opens the installed version even if another is available', async () => {
    const { client, focus, update } = setup(installed, Promise.resolve(chrome(true)))
    const notice = await screen.findByRole('dialog', { name: title })
    expect(client.acknowledgeUpdateInstalled).toHaveBeenCalledExactlyOnceWith('0.2.0')
    expect(client.openExternalLink).not.toHaveBeenCalled()

    focus(false)
    update({ ...installed, phase: 'downloading', version: '0.3.0', current: '0.2.0', notes: null, date: null, downloaded: 0, total: null })
    focus(true)
    expect(screen.getAllByRole('dialog', { name: title })).toHaveLength(1)
    expect(client.acknowledgeUpdateInstalled).toHaveBeenCalledTimes(1)

    await userEvent.click(within(notice).getByRole('button', { name: '更新说明' }))
    expect(client.openExternalLink).toHaveBeenCalledExactlyOnceWith('https://github.com/GuqierMcl/MailX/releases/tag/v0.2.0')
  })

  it('does not present or acknowledge while in the background, even on a late focused snapshot', async () => {
    let resolve!: (value: WindowChrome) => void
    const pending = new Promise<WindowChrome>((done) => { resolve = done })
    const { client, host, focus } = setup(installed, pending)
    await waitFor(() => expect(host.getChrome).toHaveBeenCalled())
    focus(false)
    await act(async () => { resolve(chrome(true)) })
    expect(screen.queryByRole('dialog', { name: title })).not.toBeInTheDocument()
    expect(client.acknowledgeUpdateInstalled).not.toHaveBeenCalled()

    focus(true)
    const notice = await screen.findByRole('dialog', { name: title })
    expect(client.acknowledgeUpdateInstalled).toHaveBeenCalledExactlyOnceWith('0.2.0')
    client.openExternalLink.mockRejectedValueOnce(new Error('browser unavailable'))
    await userEvent.click(within(notice).getByRole('button', { name: '更新说明' }))
    expect(await screen.findByText('无法打开更新说明。', { selector: 'h2' })).toBeInTheDocument()
  })

  it('does not treat startup alone as a successful update', async () => {
    const { client, host } = setup({ canInstall: true, phase: 'idle' }, Promise.resolve(chrome(true)))
    await act(async () => { await client.getUpdateState.mock.results[0].value })
    expect(host.getChrome).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog', { name: title })).not.toBeInTheDocument()
    expect(client.acknowledgeUpdateInstalled).not.toHaveBeenCalled()
  })
})
