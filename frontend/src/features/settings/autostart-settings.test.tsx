import { act, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { MailxClientError } from '@/lib/mailx-client'
import { AutostartSettings } from './autostart-settings'

describe('login startup settings', () => {
  it('defaults off, gates silent startup and preserves that preference when autostart is disabled', async () => {
    const client = testClient()
    render(<AutostartSettings client={client} />)
    const launch = screen.getByRole('switch', { name: '开机自启动' })
    const silent = screen.getByRole('switch', { name: '静默启动' })
    await waitFor(() => expect(launch).not.toHaveAttribute('aria-disabled', 'true'))
    expect(launch).not.toBeChecked()
    expect(silent).not.toBeChecked()
    expect(silent).toHaveAttribute('aria-disabled', 'true')
    expect(client.setAutostart).not.toHaveBeenCalled()
    await userEvent.click(launch)
    await waitFor(() => expect(silent).not.toHaveAttribute('aria-disabled', 'true'))
    expect(client.setAutostart).toHaveBeenCalledWith(true)
    await userEvent.click(silent)
    await waitFor(() => expect(silent).toBeChecked())
    expect(client.updateSettings).toHaveBeenCalledWith({ general: expect.objectContaining({ silentStart: true, closeBehavior: 'ask' }) })
    await waitFor(() => expect(silent).not.toHaveAttribute('aria-disabled', 'true'))
    await userEvent.click(launch)
    await waitFor(() => expect(launch).not.toBeChecked())
    expect(silent).toHaveAttribute('aria-disabled', 'true')
    expect(silent).toBeChecked()
    await userEvent.click(launch)
    await waitFor(() => expect(silent).not.toHaveAttribute('aria-disabled', 'true'))
    expect(silent).toBeChecked()
  })

  it('refreshes externally changed OS status when the app gets focus', async () => {
    const client = testClient()
    client.getAutostart.mockResolvedValue({ enabled: true, available: true })
    render(<AutostartSettings client={client} />)
    const launch = screen.getByRole('switch', { name: '开机自启动' })
    await waitFor(() => expect(launch).toBeChecked())
    client.getAutostart.mockResolvedValue({ enabled: false, available: true })
    act(() => { window.dispatchEvent(new Event('focus')) })
    await waitFor(() => expect(launch).not.toBeChecked())
    expect(client.setAutostart).not.toHaveBeenCalled()
    expect(screen.getByRole('switch', { name: '静默启动' })).toHaveAttribute('aria-disabled', 'true')
  })

  it('reports unreadable state as unknown and allows retry', async () => {
    const client = testClient()
    client.getAutostart.mockRejectedValue(new Error('OS error'))
    render(<AutostartSettings client={client} />)
    expect(await screen.findByText('当前状态无法确认。')).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: '开机自启动' })).toHaveAttribute('aria-disabled', 'true')
    client.getAutostart.mockResolvedValue({ enabled: true, available: true })
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    await waitFor(() => expect(screen.getByRole('switch', { name: '开机自启动' })).toBeChecked())
  })

  it('reads the actual state again after a failed mutation and shows a controlled error', async () => {
    const client = testClient()
    client.setAutostart.mockImplementation(async () => {
      client.getAutostart.mockResolvedValue({ enabled: true, available: true })
      throw new MailxClientError({ code: 'storageUnavailable', field: 'autostart', message: '/private/untrusted OS text' })
    })
    render(<AutostartSettings client={client} />)
    const launch = screen.getByRole('switch', { name: '开机自启动' })
    await waitFor(() => expect(launch).not.toHaveAttribute('aria-disabled', 'true'))
    await userEvent.click(launch)
    expect(await screen.findByText('无法修改开机自启动设置')).toBeInTheDocument()
    await waitFor(() => expect(launch).toBeChecked())
    expect(screen.queryByText(/untrusted OS text/)).not.toBeInTheDocument()
  })

  it('disables OS registration controls in development or preview', async () => {
    const client = testClient()
    client.getAutostart.mockResolvedValue({ enabled: false, available: false })
    render(<AutostartSettings client={client} />)
    expect(await screen.findByText(/请在安装后的 MailX 中设置开机自启动/)).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: '开机自启动' })).toHaveAttribute('aria-disabled', 'true')
    expect(screen.getByRole('switch', { name: '静默启动' })).toHaveAttribute('aria-disabled', 'true')
  })

  it('shows a failed silent preference save and retains the committed value', async () => {
    const client = testClient()
    client.getAutostart.mockResolvedValue({ enabled: true, available: true })
    client.updateSettings.mockRejectedValue(new Error('storage failed'))
    render(<AutostartSettings client={client} />)
    const silent = screen.getByRole('switch', { name: '静默启动' })
    await waitFor(() => expect(silent).not.toHaveAttribute('aria-disabled', 'true'))
    await userEvent.click(silent)
    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(silent).not.toBeChecked()
  })
})
