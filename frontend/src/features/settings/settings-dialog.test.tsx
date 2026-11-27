import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import { MailxClientError } from '@/lib/mailx-client'
import { SettingsDialog, type SettingsTarget } from './settings-dialog'
import { DEFAULT_BADGE, DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_NOW } from '@/features/now/modules/defaults'
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

  it('shows what the assistant may do and the custom instructions on the AI page', async () => {
    const client = testClient()
    client.aiState.mockResolvedValue({ connections: [], defaultModel: null, translationModel: null, route: { kind: 'direct' }, proxyPasswordSet: false })
    const { dialog } = renderDialog(client, { section: 'ai' })
    for (const name of ['整理邮件', 'AI 创建的草稿', '发送邮件']) expect(await within(dialog).findByRole('combobox', { name })).toBeInTheDocument()
    expect(within(dialog).getByRole('textbox', { name: '自定义指令' })).toBeInTheDocument()
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
    for (const name of ['外观', '发信', '联系人', '增强', '安全性', '关于']) expect(within(dialog).getByRole('button', { name })).toBeInTheDocument()
  })

  it('places verification-code cleanup on the mail enhancements page in the mail group', async () => {
    const { dialog } = renderDialog(testClient(), { section: 'general' })
    expect(within(dialog).queryByRole('switch', { name: '复制验证码后移到已删除' })).not.toBeInTheDocument()
    const mailGroup = within(dialog).getByTestId('settings-nav-group-mail')
    await userEvent.click(within(mailGroup).getByRole('button', { name: '增强' }))
    expect(within(dialog).getByRole('heading', { name: '增强' })).toBeInTheDocument()
    expect(await within(dialog).findByRole('switch', { name: '复制验证码后移到已删除' })).not.toBeChecked()
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
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ compose: { defaultAccountId: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', undoSendSeconds: 0, openMode: 'window' } }))

    await userEvent.click(sender)
    await userEvent.click(await screen.findByRole('option', { name: '跟随当前邮箱' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ compose: { defaultAccountId: '', undoSendSeconds: 0, openMode: 'window' } }))

    const undo = within(dialog).getByRole('combobox', { name: '撤销发送' })
    expect(undo).toHaveTextContent('关闭（立即发送）')
    await userEvent.click(undo)
    for (const name of ['5 秒', '10 秒', '20 秒', '30 秒']) expect(await screen.findByRole('option', { name })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('option', { name: '20 秒' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ compose: { defaultAccountId: '', undoSendSeconds: 20, openMode: 'window' } }))
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

  it('enables date grouping with Monday by default and remembers a different week start when disabled', async () => {
    const { client } = renderDialog()
    const toggle = await screen.findByRole('switch', { name: '按日期分组' })
    const weekStart = screen.getByRole('combobox', { name: '每周第一天' })
    expect(toggle).toBeChecked()
    expect(weekStart).toHaveTextContent('周一')

    await userEvent.click(weekStart)
    expect(await screen.findAllByRole('option')).toHaveLength(7)
    await userEvent.click(screen.getByRole('option', { name: '周日' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({
      appearance: { ...DEFAULT_APPEARANCE, weekStartsOn: 'sunday' },
    }))
    await userEvent.click(toggle)
    await waitFor(() => expect(weekStart).toBeDisabled())
    expect(weekStart).toHaveTextContent('周日')
    expect(client.updateSettings).toHaveBeenLastCalledWith({
      appearance: { ...DEFAULT_APPEARANCE, weekStartsOn: 'sunday', groupByDate: false },
    })
  })

  it('starts with split view and patches only the reading mode', async () => {
    const { client } = renderDialog()
    const layout = await screen.findByRole('combobox', { name: '邮件布局' })
    expect(layout).toHaveTextContent('分屏')
    await userEvent.click(layout)
    await userEvent.click(await screen.findByRole('option', { name: '单栏视图' }))
    await waitFor(() => expect(layout).toHaveTextContent('单栏视图'))
    expect(client.updateSettings).toHaveBeenLastCalledWith({ layout: { readingMode: 'single' } })
  })

  it('restores the previous reading mode if the save fails', async () => {
    const client = testClient()
    client.updateSettings.mockRejectedValue(new Error('io'))
    renderDialog(client)
    const layout = await screen.findByRole('combobox', { name: '邮件布局' })
    await userEvent.click(layout)
    await userEvent.click(await screen.findByRole('option', { name: '单栏视图' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('未能保存设置')
    expect(layout).toHaveTextContent('分屏')
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
    vi.mocked(client.getSettings).mockResolvedValue({ effectiveLocale: 'zh-CN', revision: 0, settings: { general: { language: 'zh-CN', autoUpdate: true, closeBehavior: 'ask', releaseUiInBackground: false, showMenuBarIcon: false, silentStart: false, singleKeyShortcuts: true, startupView: 'inbox' }, mail: { trashVerificationMailAfterCopy: false, syncWindow: 'days30' }, appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask', senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, badge: DEFAULT_BADGE, now: DEFAULT_NOW, compose: { defaultAccountId: '', undoSendSeconds: 0, openMode: 'window' }, mcp: { enabled: false }, assistant: { enabled: true, model: null, consented: null, excludedAccounts: [], organize: 'direct' as const, aiDrafts: 'direct' as const, send: 'confirm' as const, instructions: '', saveHistory: true, readHtml: false, summary: { auto: false, senders: [], style: 'brief' as const, language: { mode: 'interface' as const }, collapsed: false } }, translation: { provider: null, microsoftRegion: '', route: { kind: 'direct' }, targetLanguage: 'auto', suggest: true, neverSuggest: [], alwaysTranslate: [], includeQuotes: false, consentedProvider: null, consentedAi: null } }, stored: false, recovered: true })
    renderDialog(client, { section: 'appearance' })
    expect(await screen.findByText(/settings\.json\.bak/)).toBeInTheDocument()
  })

  it('lets the user configure the global default history sync window', async () => {
    const client = testClient()
    renderDialog(client, { section: 'mailEnhancements' })
    const select = await screen.findByRole('combobox', { name: '默认同步范围' })
    expect(select).toHaveTextContent('最近 30 天')
    await userEvent.click(select)
    await userEvent.click(await screen.findByRole('option', { name: '最近 90 天' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({
      mail: { trashVerificationMailAfterCopy: false, syncWindow: 'days90' },
    }))
  })
})
