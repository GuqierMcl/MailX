import { render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { LifecycleContext } from '@/features/lifecycle/lifecycle-context'
import type { LifecycleClient, LifecycleSnapshot } from '@/features/lifecycle/lifecycle-client'
import { GeneralPane } from './general-pane'

function show(patch: Partial<LifecycleSnapshot> = {}) {
  const client = testClient()
  const lifecycle: LifecycleClient = {
    getSnapshot: vi.fn(), onSnapshot: vi.fn(), resolveClose: vi.fn(),
    quit: vi.fn().mockResolvedValue(undefined), cancelQuit: vi.fn(), confirmQuit: vi.fn(), onDesktopAction: vi.fn(), setDesktopReady: vi.fn(),
  }
  const snapshot: LifecycleSnapshot = { revision: 0, platform: 'windows', backgroundAvailable: true, closeRequestId: null, quitRequestId: null, quitting: false, quitTimedOut: false, ...patch }
  render(<LifecycleContext.Provider value={{ client: lifecycle, snapshot }}><GeneralPane client={client} /></LifecycleContext.Provider>)
  return { client, lifecycle }
}

describe('general window behavior settings', () => {
  it('defaults to asking and saves a close preference while preserving other general settings', async () => {
    const { client, lifecycle } = show()
    const select = await screen.findByRole('combobox', { name: '关闭主窗口时' })
    await waitFor(() => expect(select).not.toBeDisabled())
    expect(select).toHaveTextContent('每次询问')
    await userEvent.click(select)
    await userEvent.click(await screen.findByRole('option', { name: '最小化到系统托盘' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ general: { language: 'zh-CN', autoCheckUpdates: true, closeBehavior: 'background', showMenuBarIcon: false } }))
    expect(lifecycle.resolveClose).not.toHaveBeenCalled()
    expect(lifecycle.quit).not.toHaveBeenCalled()
    await waitFor(() => expect(select).not.toBeDisabled())
    await userEvent.click(select)
    await userEvent.click(await screen.findByRole('option', { name: '每次询问' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ general: { language: 'zh-CN', autoCheckUpdates: true, closeBehavior: 'ask', showMenuBarIcon: false } }))
  })

  it('disables background hiding when Linux has no available tray', async () => {
    show({ platform: 'linux', backgroundAvailable: false })
    const select = await screen.findByRole('combobox', { name: '关闭主窗口时' })
    await waitFor(() => expect(select).not.toBeDisabled())
    expect(screen.getByText(/当前无法使用系统托盘/)).toBeInTheDocument()
    await userEvent.click(select)
    expect(await screen.findByRole('option', { name: '最小化到系统托盘' })).toHaveAttribute('aria-disabled', 'true')
    expect(screen.getByRole('option', { name: '退出应用' })).not.toHaveAttribute('aria-disabled', 'true')
  })

  it('offers a separate macOS menu bar preference without requiring it for background activity', async () => {
    const { client } = show({ platform: 'macos' })
    const toggle = await screen.findByRole('switch', { name: '在菜单栏显示 MailX' })
    await waitFor(() => expect(toggle).not.toBeDisabled())
    expect(toggle).not.toBeChecked()
    await userEvent.click(toggle)
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ general: { language: 'zh-CN', autoCheckUpdates: true, closeBehavior: 'ask', showMenuBarIcon: true } }))
    const select = screen.getByRole('combobox', { name: '关闭主窗口时' })
    await waitFor(() => expect(select).not.toBeDisabled())
    await userEvent.click(select)
    expect(await screen.findByRole('option', { name: '关闭窗口并继续运行' })).toBeInTheDocument()
  })

  it('offers explicit quit even without a tray and bypasses the close choice', async () => {
    const { lifecycle } = show({ platform: 'linux', backgroundAvailable: false })
    await userEvent.click(await screen.findByRole('button', { name: '退出 MailX' }))
    expect(lifecycle.quit).toHaveBeenCalledOnce()
    expect(lifecycle.resolveClose).not.toHaveBeenCalled()
    expect(screen.queryByRole('switch', { name: '在菜单栏显示 MailX' })).not.toBeInTheDocument()
  })
})
