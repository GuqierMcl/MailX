import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { SettingsDialog } from './settings-dialog'

const work = exampleAccount()
const home = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人', emailAddress: 'lin@x.io' })

function renderPane(client = testClient([work, home])) {
  client.listFolders.mockResolvedValue([])
  render(<SettingsDialog client={client} target={{ section: 'notifications' }} onClose={() => undefined} />)
  return { client, dialog: screen.getByRole('dialog', { name: '设置' }) }
}

const lastNotifications = (client: ReturnType<typeof testClient>) =>
  client.updateSettings.mock.lastCall?.[0].notifications

describe('notification settings', () => {
  it('starts on, with previews, quiet while in front and the system sound', async () => {
    const { dialog } = renderPane()
    expect(await within(dialog).findByRole('switch', { name: '显示通知' })).toBeChecked()
    expect(within(dialog).getByRole('switch', { name: '显示发件人和主题' })).toBeChecked()
    expect(within(dialog).getByRole('switch', { name: 'MailX 在前台时也通知' })).not.toBeChecked()
    expect(within(dialog).getByRole('combobox', { name: '提示音' })).toHaveTextContent('系统默认')
    // The system sound is the system's to play: nothing to preview here.
    expect(within(dialog).queryByRole('button', { name: '试听' })).not.toBeInTheDocument()
  })

  it('saves the whole section with each change and disables the rest when turned off', async () => {
    const { client, dialog } = renderPane()
    await userEvent.click(await within(dialog).findByRole('switch', { name: 'MailX 在前台时也通知' }))
    expect(lastNotifications(client)).toEqual({ ...DEFAULT_NOTIFICATIONS, whenFocused: true })

    await userEvent.click(within(dialog).getByRole('switch', { name: '显示通知' }))
    expect(lastNotifications(client)).toEqual({ ...DEFAULT_NOTIFICATIONS, whenFocused: true, enabled: false })
    // Base UI switches are spans: disabled shows as aria-disabled.
    await waitFor(() => expect(within(dialog).getByRole('switch', { name: '显示发件人和主题' })).toHaveAttribute('aria-disabled', 'true'))
    expect(within(dialog).getByRole('button', { name: '发送测试通知' })).toBeDisabled()
  })

  it('plays a chosen MailX sound right away and offers to play it again', async () => {
    const { client, dialog } = renderPane()
    await userEvent.click(await within(dialog).findByRole('combobox', { name: '提示音' }))
    await userEvent.click(await screen.findByRole('option', { name: '清铃' }))
    expect(lastNotifications(client)?.sound).toBe('chime')
    expect(client.previewSound).toHaveBeenCalledWith('chime')
    await userEvent.click(await within(dialog).findByRole('button', { name: '试听' }))
    expect(client.previewSound).toHaveBeenCalledTimes(2)

    // Choosing silence or the system sound plays nothing here.
    await userEvent.click(within(dialog).getByRole('combobox', { name: '提示音' }))
    await userEvent.click(await screen.findByRole('option', { name: '无声' }))
    expect(lastNotifications(client)?.sound).toBe('none')
    expect(client.previewSound).toHaveBeenCalledTimes(2)
  })

  it('mutes and unmutes single accounts by id', async () => {
    const { client, dialog } = renderPane()
    const personal = await within(dialog).findByRole('switch', { name: '个人' })
    expect(personal).toBeChecked()
    await userEvent.click(personal)
    expect(lastNotifications(client)?.mutedAccounts).toEqual([home.id])
    await waitFor(() => expect(personal).not.toBeChecked())
    await userEvent.click(within(dialog).getByRole('switch', { name: '工作邮箱' }))
    expect(lastNotifications(client)?.mutedAccounts).toEqual([work.id, home.id].sort())
    await userEvent.click(personal)
    expect(lastNotifications(client)?.mutedAccounts).toEqual([work.id])
  })

  it('sends a test notification and says whether it went out', async () => {
    const { client, dialog } = renderPane()
    await userEvent.click(await within(dialog).findByRole('button', { name: '发送测试通知' }))
    expect(client.testNotification).toHaveBeenCalledTimes(1)
    expect(await within(dialog).findByText('已发送')).toBeInTheDocument()

    client.testNotification.mockRejectedValueOnce(new Error('denied'))
    await userEvent.click(within(dialog).getByRole('button', { name: '发送测试通知' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('发送失败')
  })

  it('puts the previous value back when saving fails', async () => {
    const client = testClient([work, home])
    client.updateSettings.mockRejectedValueOnce(new Error('disk full'))
    const { dialog } = renderPane(client)
    const preview = await within(dialog).findByRole('switch', { name: '显示发件人和主题' })
    await userEvent.click(preview)
    await waitFor(() => expect(preview).toBeChecked())
    expect(within(dialog).getByText('设置保存失败')).toBeInTheDocument()
  })
})
