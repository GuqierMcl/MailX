import { fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
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

    await userEvent.click(within(dialog).getByRole('button', { name: '通用' }))
    const confirm = await screen.findByRole('alertdialog')
    await userEvent.click(within(confirm).getByRole('button', { name: '继续编辑' }))
    expect(within(dialog).getByLabelText('账号名称')).toHaveValue('改过的名称')

    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' })
    await userEvent.click(await screen.findByRole('button', { name: '放弃修改' }))
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('switches and remembers the appearance', async () => {
    renderDialog(testClient(), { section: 'general' })
    await userEvent.click(screen.getByRole('radio', { name: '深色' }))
    expect(document.documentElement).toHaveClass('dark')
    expect(localStorage.getItem('mailx.theme')).toBe('dark')
    await userEvent.click(screen.getByRole('radio', { name: '跟随系统' }))
    expect(document.documentElement).not.toHaveClass('dark')
    expect(localStorage.getItem('mailx.theme')).toBeNull()
  })
})
