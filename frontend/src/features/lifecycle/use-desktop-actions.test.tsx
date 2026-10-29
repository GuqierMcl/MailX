import { act, render, waitFor } from '@/test/render'
import { describe, expect, it, vi } from 'vitest'
import type { DesktopAction, LifecycleClient } from './lifecycle-client'
import { LifecycleContext } from './lifecycle-context'
import { useDesktopActions } from './use-desktop-actions'

function Receiver({ handle }: { handle: (action: DesktopAction) => void }) {
  useDesktopActions(handle)
  return null
}

function client() {
  return {
    getSnapshot: vi.fn<LifecycleClient['getSnapshot']>(), onSnapshot: vi.fn<LifecycleClient['onSnapshot']>(),
    resolveClose: vi.fn<LifecycleClient['resolveClose']>(), quit: vi.fn<LifecycleClient['quit']>(), cancelQuit: vi.fn<LifecycleClient['cancelQuit']>(), confirmQuit: vi.fn<LifecycleClient['confirmQuit']>(),
    onDesktopAction: vi.fn<LifecycleClient['onDesktopAction']>(),
    setDesktopReady: vi.fn<LifecycleClient['setDesktopReady']>().mockResolvedValue(undefined),
  }
}

describe('tray action readiness', () => {
  it('enables tray actions only after subscribing and disables them on unmount', async () => {
    const lifecycle = client()
    let subscribed!: (unsubscribe: () => void) => void
    lifecycle.onDesktopAction.mockImplementation(() => new Promise((resolve) => { subscribed = resolve }))
    const handle = vi.fn()
    const stop = vi.fn()
    const { unmount } = render(<LifecycleContext.Provider value={{ client: lifecycle, snapshot: null }}><Receiver handle={handle} /></LifecycleContext.Provider>)
    expect(lifecycle.setDesktopReady).not.toHaveBeenCalled()
    await act(async () => subscribed(stop))
    expect(lifecycle.setDesktopReady).toHaveBeenCalledWith(true)
    act(() => lifecycle.onDesktopAction.mock.calls[0][0]('settings'))
    expect(handle).toHaveBeenCalledWith('settings')
    unmount()
    expect(stop).toHaveBeenCalledOnce()
    expect(lifecycle.setDesktopReady).toHaveBeenLastCalledWith(false)
    act(() => lifecycle.onDesktopAction.mock.calls[0][0]('compose'))
    expect(handle).toHaveBeenCalledTimes(1)
  })

  it('does not mark an already unmounted workbench ready when its subscription arrives late', async () => {
    const lifecycle = client()
    let subscribed!: (unsubscribe: () => void) => void
    lifecycle.onDesktopAction.mockImplementation(() => new Promise((resolve) => { subscribed = resolve }))
    const stop = vi.fn()
    const { unmount } = render(<LifecycleContext.Provider value={{ client: lifecycle, snapshot: null }}><Receiver handle={() => undefined} /></LifecycleContext.Provider>)
    unmount()
    await act(async () => subscribed(stop))
    await waitFor(() => expect(stop).toHaveBeenCalledOnce())
    expect(lifecycle.setDesktopReady).not.toHaveBeenCalled()
  })
})
