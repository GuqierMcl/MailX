import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import { MailxClientError } from '@/lib/mailx-client'
import { SettingsDialog, type SettingsTarget } from './settings-dialog'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'

function renderDialog(client = testClient(), target: SettingsTarget = { section: 'appearance' }) {
  const onClose = vi.fn()
  render(<SettingsDialog client={client} target={target} onClose={onClose} />)
  return { client, onClose, dialog: screen.getByRole('dialog', { name: '设置' }) }
}

afterEach(() => {
  localStorage.clear()
  document.documentElement.classList.remove('dark')
})

describe('settings dialog', () => {
  it('offers all remote image policies on the security page and saves only that section', async () => {
    const { client, dialog } = renderDialog(testClient(), { section: 'security' })
    const select = await within(dialog).findByRole('combobox', { name: '邮件里的网络图片' })
    expect(select).toHaveTextContent('每次询问')
    await userEvent.click(select)
    expect(await screen.findByRole('option', { name: '自动加载' })).toBeInTheDocument()
    await userEvent.click(await screen.findByRole('option', { name: '永不加载' }))
    await waitFor(() => expect(select).toHaveTextContent('永不加载'))
    expect(client.updateSettings).toHaveBeenCalledWith({ security: { remoteImages: 'never', senderAvatars: false } })
  })

  it('keeps sender avatar lookups off until the avatar switch is turned on', async () => {
    const { client } = renderDialog(testClient(), { section: 'security' })
    const toggle = await screen.findByRole('switch', { name: '显示发件人头像' })
    expect(toggle).not.toBeChecked()
    await userEvent.click(toggle)
    expect(client.updateSettings).toHaveBeenLastCalledWith({ security: { remoteImages: 'ask', senderAvatars: true } })
  })

  it('clears the image cache on request and reports the result', async () => {
    const { client } = renderDialog(testClient(), { section: 'security' })
    await userEvent.click(await screen.findByRole('button', { name: '清除缓存' }))
    expect(client.clearImageCache).toHaveBeenCalledTimes(1)
    expect(await screen.findByText('已清除')).toBeInTheDocument()
  })

  it('says so when the image cache could not be cleared', async () => {
    const client = testClient()
    client.clearImageCache.mockRejectedValue(new Error('io'))
    renderDialog(client, { section: 'security' })
    await userEvent.click(await screen.findByRole('button', { name: '清除缓存' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('清除失败')
  })

  it('shows a visible title and keeps navigation on narrow windows', () => {
    const { dialog } = renderDialog()
    expect(within(dialog).getByRole('heading', { name: '设置' })).toBeVisible()
    for (const name of ['外观', '发信', '安全性', '关于']) expect(within(dialog).getByRole('button', { name })).toBeInTheDocument()
  })

  it('has a 发信 section with the default sender account and the undo window', async () => {
    const rows = [exampleAccount(), exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人邮箱', emailAddress: 'me@example.com' })]
    const client = testClient(rows)
    client.listFolders.mockResolvedValue([])
    const { dialog } = renderDialog(client, { section: 'compose' })
    expect(within(dialog).getByRole('button', { name: '发信' })).toBeInTheDocument()

    const sender = await within(dialog).findByRole('combobox', { name: '默认发件账号' })
    expect(sender).toHaveTextContent('跟随当前邮箱')
    await userEvent.click(sender)
    await userEvent.click(await screen.findByRole('option', { name: /个人邮箱/ }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ compose: { defaultAccountId: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', undoSendSeconds: 0 } }))

    await userEvent.click(sender)
    await userEvent.click(await screen.findByRole('option', { name: '跟随当前邮箱' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ compose: { defaultAccountId: '', undoSendSeconds: 0 } }))

    const undo = within(dialog).getByRole('combobox', { name: '撤销发送' })
    expect(undo).toHaveTextContent('关闭（立即发送）')
    await userEvent.click(undo)
    for (const name of ['5 秒', '10 秒', '20 秒', '30 秒']) expect(await screen.findByRole('option', { name })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('option', { name: '20 秒' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ compose: { defaultAccountId: '', undoSendSeconds: 20 } }))
    await waitFor(() => expect(undo).toHaveTextContent('20 秒'))
  })

  it('rolls the undo choice back when the core refuses it', async () => {
    const client = testClient()
    client.updateSettings.mockRejectedValue(new MailxClientError({ code: 'validation', field: 'compose.undoSendSeconds', message: '不支持的撤销时间。' }))
    renderDialog(client, { section: 'compose' })
    const undo = await screen.findByRole('combobox', { name: '撤销发送' })
    await userEvent.click(undo)
    await userEvent.click(await screen.findByRole('option', { name: '10 秒' }))
    expect(await screen.findByText('设置保存失败')).toBeInTheDocument()
    await waitFor(() => expect(undo).toHaveTextContent('关闭（立即发送）'))
  })

  it('saves appearance choices through the client', async () => {
    const client = testClient()
    renderDialog(client, { section: 'appearance' })
    await userEvent.click(await screen.findByRole('radio', { name: '深色' }))
    expect(client.updateSettings).toHaveBeenLastCalledWith({ appearance: { ...DEFAULT_APPEARANCE, theme: 'dark' } })
    expect(await screen.findByRole('radio', { name: '深色' })).toBeChecked()

    await userEvent.click(screen.getByRole('radio', { name: '紫' }))
    expect(client.updateSettings).toHaveBeenLastCalledWith({ appearance: expect.objectContaining({ theme: 'dark', accentColor: '#7c3aed' }) })
  })

  it('commits the font list on Enter, normalized and in order, and clears it back to the default', async () => {
    const client = testClient()
    renderDialog(client, { section: 'appearance' })
    const input = await screen.findByLabelText('界面字体')
    await userEvent.type(input, `  'Noto Serif SC', , Georgia{Enter}`)
    expect(client.updateSettings).toHaveBeenLastCalledWith({
      appearance: { ...DEFAULT_APPEARANCE, interfaceFont: 'Noto Serif SC, Georgia' },
    })
    expect(await screen.findByLabelText('界面字体')).toHaveValue('Noto Serif SC, Georgia')

    await userEvent.click(screen.getByRole('button', { name: '恢复默认' }))
    expect(client.updateSettings).toHaveBeenLastCalledWith({ appearance: DEFAULT_APPEARANCE })
  })

  it('saves the mail body font separately from the interface font', async () => {
    const client = testClient()
    renderDialog(client, { section: 'appearance' })
    await userEvent.type(await screen.findByLabelText('邮件字体'), 'Georgia{Enter}')
    expect(client.updateSettings).toHaveBeenLastCalledWith({
      appearance: { ...DEFAULT_APPEARANCE, mailFont: 'Georgia' },
    })
  })

  it('saves the mail body dark mode choice', async () => {
    const client = testClient()
    renderDialog(client, { section: 'appearance' })
    await userEvent.click(await screen.findByRole('combobox', { name: '深色模式下的邮件' }))
    await userEvent.click(await screen.findByRole('option', { name: '始终白底' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({
      appearance: { ...DEFAULT_APPEARANCE, mailTheme: 'light' },
    }))
  })

  it('offers list preview lines, one by default', async () => {
    const client = testClient()
    renderDialog(client, { section: 'appearance' })
    const select = await screen.findByRole('combobox', { name: '邮件预览行数' })
    expect(select).toHaveTextContent('1 行')
    await userEvent.click(select)
    for (const name of ['不显示', '2 行', '3 行']) expect(await screen.findByRole('option', { name })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('option', { name: '3 行' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({
      appearance: { ...DEFAULT_APPEARANCE, previewLines: 'three' },
    }))
  })

  it('keeps a rejected font list in the field with an error', async () => {
    const client = testClient()
    vi.mocked(client.updateSettings).mockRejectedValue(
      new MailxClientError({ code: 'validation', message: '输入内容无效。', field: 'appearance.interfaceFont' }),
    )
    renderDialog(client, { section: 'appearance' })
    await userEvent.type(await screen.findByLabelText('界面字体'), 'Bad;Font{Enter}')
    expect(await screen.findByText(/字体名里有不能使用的字符/)).toBeInTheDocument()
    expect(screen.getByLabelText('界面字体')).toHaveValue('Bad;Font')
    expect(screen.getByLabelText('界面字体')).toHaveAttribute('aria-invalid', 'true')
  })

  it('explains a settings file that had to be moved aside', async () => {
    const client = testClient()
    vi.mocked(client.getSettings).mockResolvedValue({ effectiveLocale: 'zh-CN', revision: 0, settings: { general: { language: 'zh-CN', autoCheckUpdates: true }, appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask', senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, compose: { defaultAccountId: '', undoSendSeconds: 0 } }, stored: false, recovered: true })
    renderDialog(client, { section: 'appearance' })
    expect(await screen.findByText(/settings\.json\.bak/)).toBeInTheDocument()
  })
})
