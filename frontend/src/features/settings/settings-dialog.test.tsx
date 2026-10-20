import { fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import { MailxClientError } from '@/lib/mailx-client'
import { SettingsDialog, type SettingsTarget } from './settings-dialog'

function renderDialog(client = testClient(), target: SettingsTarget = { section: 'accounts' }) {
  const onClose = vi.fn()
  const onAccountsChanged = vi.fn()
  render(<SettingsDialog client={client} target={target} onClose={onClose} onAccountsChanged={onAccountsChanged} />)
  return { client, onClose, onAccountsChanged, dialog: screen.getByRole('dialog', { name: '设置' }) }
}

afterEach(() => {
  localStorage.clear()
  document.documentElement.classList.remove('dark')
})

describe('settings dialog', () => {
  it('offers all remote image policies on the security page and saves only that section', async () => {
    const { client, dialog } = renderDialog(testClient(), { section: 'security' })
    const select = await within(dialog).findByRole('combobox', { name: '加载远程图片' })
    expect(select).toHaveTextContent('每次询问')
    await userEvent.click(select)
    expect(screen.getByRole('option', { name: '自动加载' })).toBeInTheDocument()
    await userEvent.click(await screen.findByRole('option', { name: '永不加载' }))
    await waitFor(() => expect(select).toHaveTextContent('永不加载'))
    expect(client.updateSettings).toHaveBeenCalledWith({ security: { remoteImages: 'never' } })
  })

  it('starts an empty account list with a new-account form', async () => {
    const { dialog } = renderDialog()
    await userEvent.click(await within(dialog).findByRole('button', { name: '添加第一个账号' }))
    expect(within(dialog).getByText('添加邮箱账号')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveValue('')
    expect(within(dialog).queryByRole('button', { name: /OAuth|连接测试|删除/ })).not.toBeInTheDocument()
  })

  it('loads an account through get and reports saves to the workbench', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    const { dialog, onAccountsChanged } = renderDialog(client)
    await userEvent.click(await within(dialog).findByRole('button', { name: /工作邮箱/ }))
    expect(client.getAccount).toHaveBeenCalledWith(account.id)
    expect(await within(dialog).findByLabelText('账号名称')).toHaveValue('工作邮箱')
    await userEvent.click(within(dialog).getByRole('button', { name: '保存更改' }))
    await waitFor(() => expect(onAccountsChanged).toHaveBeenCalledOnce())
  })

  it('confirms before discarding unsaved edits', async () => {
    const client = testClient([exampleAccount()])
    const { dialog, onClose } = renderDialog(client)
    await userEvent.click(await within(dialog).findByRole('button', { name: /工作邮箱/ }))
    fireEvent.change(await within(dialog).findByLabelText('账号名称'), { target: { value: '改过的名称' } })

    await userEvent.click(within(dialog).getByRole('button', { name: '外观' }))
    const confirm = await screen.findByRole('alertdialog')
    await userEvent.click(within(confirm).getByRole('button', { name: '继续编辑' }))
    expect(within(dialog).getByLabelText('账号名称')).toHaveValue('改过的名称')

    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' })
    await userEvent.click(await screen.findByRole('button', { name: '放弃修改' }))
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('shows a visible title and keeps navigation on narrow windows', () => {
    const { dialog } = renderDialog()
    expect(within(dialog).getByRole('heading', { name: '设置' })).toBeVisible()
    for (const name of ['外观', '账号', '关于']) expect(within(dialog).getByRole('button', { name })).toBeInTheDocument()
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
    await userEvent.type(await screen.findByLabelText('邮件内容字体'), 'Georgia{Enter}')
    expect(client.updateSettings).toHaveBeenLastCalledWith({
      appearance: { ...DEFAULT_APPEARANCE, mailFont: 'Georgia' },
    })
  })

  it('saves the mail body dark mode choice', async () => {
    const client = testClient()
    renderDialog(client, { section: 'appearance' })
    await userEvent.click(await screen.findByRole('combobox', { name: '邮件内容深色适配' }))
    await userEvent.click(await screen.findByRole('option', { name: '始终白底' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({
      appearance: { ...DEFAULT_APPEARANCE, mailTheme: 'light' },
    }))
  })

  it('keeps a rejected font list in the field with an error', async () => {
    const client = testClient()
    vi.mocked(client.updateSettings).mockRejectedValue(
      new MailxClientError({ code: 'validation', message: '输入内容无效。', field: 'appearance.interfaceFont' }),
    )
    renderDialog(client, { section: 'appearance' })
    await userEvent.type(await screen.findByLabelText('界面字体'), 'Bad;Font{Enter}')
    expect(await screen.findByText(/字体名不能包含引号/)).toBeInTheDocument()
    expect(screen.getByLabelText('界面字体')).toHaveValue('Bad;Font')
    expect(screen.getByLabelText('界面字体')).toHaveAttribute('aria-invalid', 'true')
  })

  it('explains a settings file that had to be moved aside', async () => {
    const client = testClient()
    vi.mocked(client.getSettings).mockResolvedValue({ settings: { appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask' } }, stored: false, recovered: true })
    renderDialog(client, { section: 'appearance' })
    expect(await screen.findByText(/settings\.json\.bak/)).toBeInTheDocument()
  })
})
