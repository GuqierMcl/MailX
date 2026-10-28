import { invoke, isTauri } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createDesktopLifecycleClient } from './lifecycle-client'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }))
beforeEach(() => { vi.clearAllMocks(); vi.mocked(isTauri).mockReturnValue(true) })

describe('desktop lifecycle boundary', () => {
  it('forwards only the selected request, choice, and explicit lifecycle actions', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: true, data: null, requestId: 'response' })
    const client = createDesktopLifecycleClient()!
    await client.getSnapshot()
    await client.resolveClose({ requestId: 3, action: 'background', remember: true })
    await client.quit()
    await client.cancelQuit()
    await client.cancelQuit(8)
    await client.confirmQuit(9)
    await client.setDesktopReady(true)
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ['app_get_lifecycle', undefined],
      ['app_resolve_close', { requestId: 3, action: 'background', remember: true }],
      ['app_quit', undefined],
      ['app_cancel_quit', { generation: null }],
      ['app_cancel_quit', { generation: 8 }],
      ['app_confirm_quit', { generation: 9 }],
      ['app_desktop_ready', { ready: true }],
    ])
  })

  it('preserves service failures and sanitizes transport failures', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ ok: false, error: { code: 'storageUnavailable', message: 'Unable to save.', field: null }, requestId: 'failed' })
    const client = createDesktopLifecycleClient()!
    await expect(client.resolveClose({ requestId: 7, action: 'quit', remember: true })).rejects.toMatchObject({ failure: { code: 'storageUnavailable' } })
    vi.mocked(invoke).mockRejectedValueOnce(new Error('/private/secret/path'))
    await expect(client.quit()).rejects.toMatchObject({ failure: { code: 'transport' } })
  })

  it('does not expose desktop controls in an ordinary browser', () => {
    vi.mocked(isTauri).mockReturnValue(false)
    expect(createDesktopLifecycleClient()).toBeNull()
    expect(invoke).not.toHaveBeenCalled()
  })
})
