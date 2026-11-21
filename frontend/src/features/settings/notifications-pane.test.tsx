import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_BADGE, DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { LifecycleContext } from '@/features/lifecycle/lifecycle-context'
import type { LifecycleClient, LifecycleSnapshot } from '@/features/lifecycle/lifecycle-client'
import { SettingsDialog } from './settings-dialog'

const work = exampleAccount()
const home = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人', emailAddress: 'lin@x.io' })

function renderPane(client = testClient([work, home]), badgeAvailable?: boolean) {
  client.listFolders.mockResolvedValue([])
  const dialog = <SettingsDialog client={client} target={{ section: 'notifications' }} onClose={() => undefined} />
  if (badgeAvailable === undefined) render(dialog)
  else {
    const lifecycle: LifecycleClient = {
      getSnapshot: vi.fn(), onSnapshot: vi.fn(), resolveClose: vi.fn(), quit: vi.fn(), cancelQuit: vi.fn(),
      confirmQuit: vi.fn(), onDesktopAction: vi.fn(), setDesktopReady: vi.fn(),
    }
    const snapshot: LifecycleSnapshot = { revision: 0, platform: 'windows', backgroundAvailable: true, badgeAvailable, closeRequestId: null, quitRequestId: null, quitting: false, quitTimedOut: false }
    render(<LifecycleContext.Provider value={{ client: lifecycle, snapshot }}>{dialog}</LifecycleContext.Provider>)
  }
  return { client, dialog: screen.getByRole('dialog', { name: '设置' }) }
}

const lastNotifications = (client: ReturnType<typeof testClient>) =>
  client.updateSettings.mock.lastCall?.[0].notifications
const lastBadge = (client: ReturnType<typeof testClient>) =>
  client.updateSettings.mock.lastCall?.[0].badge

describe('notification settings', () => {
  it('lists account notification controls in the saved display order', async () => {
    const client = testClient([work, home])
    await client.updateSettings({ layout: { accountOrder: [home.id, work.id] } })
    const { dialog } = renderPane(client)
    const group = await within(dialog).findByRole('group', { name: '按账号' })
    const homeControl = await within(group).findByRole('switch', { name: home.displayName })
    const workControl = within(group).getByRole('switch', { name: work.displayName })
    const switches = within(group).getAllByRole('switch').filter((element) => element === homeControl || element === workControl)
    expect(switches).toEqual([homeControl, workControl])
  })

  it('starts on, with previews, quiet while in front and the system sound', async () => {
    const { dialog } = renderPane()
    expect(await within(dialog).findByRole('switch', { name: '收到新邮件时提醒我' })).toBeChecked()
    expect(within(dialog).getByRole('switch', { name: '显示通知内容' })).toBeChecked()
    expect(within(dialog).getByRole('switch', { name: '正在使用时也通知' })).not.toBeChecked()
    expect(within(dialog).getByRole('combobox', { name: '提示音' })).toHaveTextContent('系统默认')
    // The system sound is the system's to play: nothing to preview here.
    expect(within(dialog).queryByRole('button', { name: '试听' })).not.toBeInTheDocument()
  })

  it('saves the whole section with each change and disables the rest when turned off', async () => {
    const { client, dialog } = renderPane()
    await userEvent.click(await within(dialog).findByRole('switch', { name: '正在使用时也通知' }))
    expect(lastNotifications(client)).toEqual({ ...DEFAULT_NOTIFICATIONS, whenFocused: true })

    await userEvent.click(within(dialog).getByRole('switch', { name: '收到新邮件时提醒我' }))
    expect(lastNotifications(client)).toEqual({ ...DEFAULT_NOTIFICATIONS, whenFocused: true, enabled: false })
    // Base UI switches are spans: disabled shows as aria-disabled.
    await waitFor(() => expect(within(dialog).getByRole('switch', { name: '显示通知内容' })).toHaveAttribute('aria-disabled', 'true'))
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
    const group = await within(dialog).findByRole('group', { name: '按账号' })
    const personal = await within(group).findByRole('switch', { name: '个人' })
    expect(personal).toBeChecked()
    await userEvent.click(personal)
    expect(lastNotifications(client)?.mutedAccounts).toEqual([home.id])
    await waitFor(() => expect(personal).not.toBeChecked())
    await userEvent.click(within(group).getByRole('switch', { name: '工作邮箱' }))
    expect(lastNotifications(client)?.mutedAccounts).toEqual([work.id, home.id].sort())
    await userEvent.click(personal)
    expect(lastNotifications(client)?.mutedAccounts).toEqual([work.id])
  })

  it('hides the app icon badge where the host cannot show one', async () => {
    const { dialog } = renderPane(testClient([work, home]), false)
    await within(dialog).findByRole('switch', { name: '收到新邮件时提醒我' })
    expect(within(dialog).queryByRole('switch', { name: '显示角标' })).not.toBeInTheDocument()
    expect(within(dialog).queryByRole('group', { name: '计入的账号' })).not.toBeInTheDocument()
  })

  it('turns the app icon badge off and on and saves which accounts it counts', async () => {
    const { client, dialog } = renderPane(testClient([work, home]), true)
    const show = await within(dialog).findByRole('switch', { name: '显示角标' })
    expect(show).toBeChecked()
    const group = await within(dialog).findByRole('group', { name: '计入的账号' })
    const personal = await within(group).findByRole('switch', { name: '个人' })
    expect(personal).toBeChecked()

    await userEvent.click(personal)
    expect(lastBadge(client)).toEqual({ ...DEFAULT_BADGE, excludedAccounts: [home.id] })
    // Notification muting is a separate list.
    expect(lastNotifications(client)).toBeUndefined()
    await waitFor(() => expect(personal).not.toBeChecked())
    await userEvent.click(within(group).getByRole('switch', { name: '工作邮箱' }))
    expect(lastBadge(client)?.excludedAccounts).toEqual([work.id, home.id].sort())
    await userEvent.click(personal)
    expect(lastBadge(client)?.excludedAccounts).toEqual([work.id])

    await userEvent.click(show)
    expect(lastBadge(client)).toEqual({ enabled: false, excludedAccounts: [work.id] })
    await waitFor(() => expect(personal).toHaveAttribute('aria-disabled', 'true'))
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
    const preview = await within(dialog).findByRole('switch', { name: '显示通知内容' })
    await userEvent.click(preview)
    await waitFor(() => expect(preview).toBeChecked())
    const group = within(dialog).getByRole('group', { name: '新邮件通知' })
    expect(within(group).getByRole('alert')).toHaveTextContent('设置保存失败')
    await userEvent.click(preview)
    await waitFor(() => expect(preview).not.toBeChecked())
    expect(within(dialog).queryByRole('alert')).not.toBeInTheDocument()
  })
})
