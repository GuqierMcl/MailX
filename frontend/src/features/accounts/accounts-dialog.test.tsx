import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { Account, AccountRemoval } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { exampleAccount, exampleSyncState, testClient } from './test-fixtures'
import { AccountsDialog } from './accounts-dialog'
import { i18n } from '@/i18n'

const second = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人邮箱', emailAddress: 'me@example.com', color: '#16a34a' })

function renderDialog(rows: Account[] = [exampleAccount()], target: { accountId?: string } = {}, client = testClient(rows), syncStates = {}) {
  const onClose = vi.fn()
  const onAccountsChanged = vi.fn()
  const onAddAccount = vi.fn()
  const view = render(<AccountsDialog client={client} target={target} accounts={rows} syncStates={syncStates} onClose={onClose} onAccountsChanged={onAccountsChanged} onAddAccount={onAddAccount} />)
  return { client, onClose, onAccountsChanged, onAddAccount, view, dialog: screen.getByRole('dialog', { name: '邮箱账号' }) }
}

describe('accounts dialog', () => {
  it('keeps unsaved profile input and updates an existing field error when switching language', async () => {
    const { client, dialog } = renderDialog()
    const name = await within(dialog).findByLabelText('账号名称')
    fireEvent.change(name, { target: { value: ' ' } })
    fireEvent.blur(name)
    expect(within(dialog).getByText('请填写账号名称。')).toBeInTheDocument()
    await act(async () => { await i18n.changeLanguage('en') })
    expect(within(dialog).getByLabelText('Account name')).toBe(name)
    expect(name).toHaveValue(' ')
    expect(within(dialog).getByText('Enter an account name.')).toBeInTheDocument()
    expect(client.updateProfile).not.toHaveBeenCalled()
  })

  it('shows the profile and a read-only connection summary', async () => {
    const account = exampleAccount()
    const { client, dialog } = renderDialog([account])
    expect(await within(dialog).findByLabelText('账号名称')).toHaveValue('工作邮箱')
    expect(client.getAccount).toHaveBeenCalledWith(account.id)
    expect(within(dialog).getByText(/IMAP · imap\.example\.com:993 · TLS/)).toBeInTheDocument()
    expect(within(dialog).getByText('直接连接')).toBeInTheDocument()
    expect(within(dialog).getByText('IMAP/SMTP')).toBeInTheDocument()
    // Connection fields are not editable here.
    expect(within(dialog).queryByLabelText('IMAP 主机')).not.toBeInTheDocument()
  })

  it('saves a renamed account on blur through the profile request only', async () => {
    const { client, dialog, onAccountsChanged } = renderDialog()
    const name = await within(dialog).findByLabelText('账号名称')
    fireEvent.change(name, { target: { value: '  新名称 ' } })
    fireEvent.blur(name)
    await waitFor(() => expect(onAccountsChanged).toHaveBeenCalledOnce())
    expect(client.updateProfile).toHaveBeenCalledWith({ accountId: exampleAccount().id, profile: { displayName: '新名称', color: null, senderName: null, useProviderIcon: true } })
    expect(client.updateConnection).not.toHaveBeenCalled()
    expect(await within(dialog).findByText('已保存')).toBeInTheDocument()
  })

  it('saves a color as soon as it is picked, keeping the saved name', async () => {
    const { client, dialog } = renderDialog()
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('radio', { name: '紫' }))
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledWith({ accountId: exampleAccount().id, profile: { displayName: '工作邮箱', color: '#7c3aed', senderName: null, useProviderIcon: true } }))
    expect(within(dialog).getByRole('radio', { name: '紫' })).toHaveAttribute('aria-checked', 'true')
  })

  it('saves provider icon choices per account without changing the connection', async () => {
    const first = exampleAccount({ providerId: 'qq', useProviderIcon: true })
    const other = exampleAccount({ ...second, providerId: 'qq', useProviderIcon: false })
    const { client, dialog } = renderDialog([first, other])
    const preference = await within(dialog).findByRole('switch', { name: '使用服务商图标' })
    expect(preference).toBeChecked()
    await userEvent.click(preference)
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledWith({
      accountId: first.id,
      profile: { displayName: first.displayName, color: first.color, senderName: first.senderName, useProviderIcon: false },
    }))
    await userEvent.click(within(dialog).getByRole('button', { name: other.displayName }))
    const otherPreference = await within(dialog).findByRole('switch', { name: '使用服务商图标' })
    expect(otherPreference).not.toBeChecked()
    await userEvent.click(otherPreference)
    await waitFor(() => expect(client.updateProfile).toHaveBeenLastCalledWith({
      accountId: other.id,
      profile: { displayName: other.displayName, color: other.color, senderName: other.senderName, useProviderIcon: true },
    }))
    expect(client.updateConnection).not.toHaveBeenCalled()
  })

  it('offers no provider icon preference for a manually configured mailbox', async () => {
    const { dialog } = renderDialog()
    await within(dialog).findByLabelText('账号名称')
    expect(within(dialog).queryByRole('switch', { name: '使用服务商图标' })).not.toBeInTheDocument()
  })

  it('saves the sender name when editing settles, trimmed, and clears it with an empty value', async () => {
    const { client, dialog } = renderDialog([exampleAccount({ senderName: '林薇' })])
    const field = await within(dialog).findByLabelText('发件人名称')
    expect(field).toHaveValue('林薇')
    expect(field).toHaveAttribute('placeholder', '收件人看到的名字，可留空')
    fireEvent.change(field, { target: { value: '  Lin Wei ' } })
    fireEvent.blur(field)
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledWith({
      accountId: exampleAccount().id, profile: { displayName: '工作邮箱', color: null, senderName: 'Lin Wei', useProviderIcon: true },
    }))
    expect(field).toHaveValue('Lin Wei')

    fireEvent.change(field, { target: { value: ' ' } })
    fireEvent.keyDown(field, { key: 'Enter' })
    await waitFor(() => expect(client.updateProfile).toHaveBeenLastCalledWith({
      accountId: exampleAccount().id, profile: { displayName: '工作邮箱', color: null, senderName: null, useProviderIcon: true },
    }))
  })

  it('shows a rejected sender name on its own field', async () => {
    const client = testClient()
    client.updateProfile.mockRejectedValue(new MailxClientError({ code: 'validation', field: 'senderName', message: '发件人名称不能包含控制字符。' }))
    const { dialog } = renderDialog([exampleAccount()], {}, client)
    const field = await within(dialog).findByLabelText('发件人名称')
    fireEvent.change(field, { target: { value: 'bad' } })
    fireEvent.blur(field)
    expect(await within(dialog).findByText('配置参数无效，请检查标记的字段。')).toBeInTheDocument()
    expect(field).toHaveAttribute('aria-invalid', 'true')
  })

  it('does not save an empty name', async () => {
    const { client, dialog } = renderDialog()
    const name = await within(dialog).findByLabelText('账号名称')
    fireEvent.change(name, { target: { value: ' ' } })
    fireEvent.blur(name)
    expect(within(dialog).getByText('请填写账号名称。')).toBeInTheDocument()
    expect(client.updateProfile).not.toHaveBeenCalled()
  })

  it('reports a failed profile save', async () => {
    const client = testClient([exampleAccount()])
    client.updateProfile.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', field: null, message: '无法写入本机数据。' }))
    const { dialog } = renderDialog([exampleAccount()], {}, client)
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('radio', { name: '红' }))
    expect(await within(dialog).findByText('资料未保存')).toBeInTheDocument()
    expect(within(dialog).getByText('无法读写本地数据，请检查可用空间与访问权限。')).toBeInTheDocument()
  })

  it('saves a name typed but not committed when switching accounts', async () => {
    const { client, dialog } = renderDialog([exampleAccount(), second])
    fireEvent.change(await within(dialog).findByLabelText('账号名称'), { target: { value: '没按回车' } })
    await userEvent.click(within(dialog).getByRole('button', { name: /个人邮箱/ }))
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledWith({ accountId: exampleAccount().id, profile: { displayName: '没按回车', color: null, senderName: null, useProviderIcon: true } }))
    await waitFor(() => expect(client.getAccount).toHaveBeenCalledWith(second.id))
  })

  it('opens the requested account', async () => {
    const { client } = renderDialog([exampleAccount(), second], { accountId: second.id })
    await waitFor(() => expect(client.getAccount).toHaveBeenCalledWith(second.id))
  })

  it('edits the connection in its own dialog and saves it through the connection request', async () => {
    const { client, dialog, onAccountsChanged } = renderDialog()
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('button', { name: '修改连接' }))
    const connection = await screen.findByRole('dialog', { name: '修改连接' })
    expect(await within(connection).findByText('IMAP/SMTP')).toBeInTheDocument()
    fireEvent.change(await within(connection).findByLabelText('IMAP 主机'), { target: { value: 'imap.moved.example' } })
    await userEvent.click(within(connection).getByRole('button', { name: '保存并重新连接' }))
    await waitFor(() => expect(client.updateConnection).toHaveBeenCalledOnce())
    expect(client.updateConnection.mock.calls[0][0]).toMatchObject({ accountId: exampleAccount().id, expectedRevision: 3, connection: { imap: { host: 'imap.moved.example' } } })
    expect(client.updateProfile).not.toHaveBeenCalled()
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '修改连接' })).not.toBeInTheDocument())
    expect(onAccountsChanged).toHaveBeenCalled()
  })

  it('points to the connection when credentials need confirming or sync stopped', async () => {
    const pending = exampleAccount({ authentication: { mode: 'shared', imap: { id: 'a', generation: 1, passwordConfigured: true, trust: 'needsConfirmation' }, smtp: { id: 'a', generation: 1, passwordConfigured: true, trust: 'needsConfirmation' } } })
    const { dialog, view } = renderDialog([pending])
    expect(await within(dialog).findByText('凭证需要重新确认')).toBeInTheDocument()
    view.unmount()

    const stopped = exampleSyncState({ phase: 'failed', failure: { code: 'authentication', message: '服务器拒绝了登录。', detail: null, willRetry: false } })
    const again = renderDialog([exampleAccount()], {}, testClient([exampleAccount()]), { [exampleAccount().id]: stopped })
    expect(await within(again.dialog).findByText('同步已停止')).toBeInTheDocument()
    expect(within(again.dialog).getByText(/邮箱服务器拒绝了账号登录/)).toBeInTheDocument()
  })

  it('shows an empty state with an add button when there are no accounts', async () => {
    const { dialog, onAddAccount } = renderDialog([])
    await userEvent.click(within(dialog).getByRole('button', { name: '添加第一个账号' }))
    expect(onAddAccount).toHaveBeenCalledOnce()
  })

  it('confirms removal beside edit connection, and cancellation leaves the account intact', async () => {
    const { client, dialog } = renderDialog()
    const remove = await within(dialog).findByRole('button', { name: '移除账号' })
    expect(remove.previousElementSibling).toHaveTextContent('修改连接')
    await userEvent.click(remove)
    const confirmation = await screen.findByRole('alertdialog')
    expect(within(confirmation).getByText('work@example.com')).toBeInTheDocument()
    expect(within(confirmation).getByText(/服务器上的邮件不会被删除/)).toBeInTheDocument()
    expect(within(confirmation).getByText('已提交或结果不确定的邮件可能已经送达，移除账号不会撤回这些邮件。')).toBeInTheDocument()
    expect(await within(confirmation).findByText('本地草稿：0 封 · 等待发送：0 封。')).toBeInTheDocument()
    await waitFor(() => expect(within(confirmation).getByRole('button', { name: '取消' })).toHaveFocus())
    await userEvent.click(within(confirmation).getByRole('button', { name: '取消' }))
    expect(client.removeAccount).not.toHaveBeenCalled()
    expect(within(dialog).getByLabelText('账号名称')).toHaveValue('工作邮箱')
  })

  it('removes the selected account and selects its neighbour before the parent refresh finishes', async () => {
    const { client, dialog } = renderDialog([exampleAccount(), second])
    await userEvent.click(await within(dialog).findByRole('button', { name: '移除账号' }))
    const confirmation = await screen.findByRole('alertdialog')
    await waitFor(() => expect(within(confirmation).getByRole('button', { name: '移除账号' })).toBeEnabled())
    await userEvent.click(within(confirmation).getByRole('button', { name: '移除账号' }))
    expect(client.removeAccount).toHaveBeenCalledWith({ accountId: exampleAccount().id, expectedRevision: 3 })
    await waitFor(() => expect(within(dialog).getByLabelText('账号名称')).toHaveValue('个人邮箱'))
    expect(within(dialog).queryByRole('button', { name: /工作邮箱/ })).not.toBeInTheDocument()
  })

  it('shows the empty state after removing the last account and does not flush its unsaved profile', async () => {
    const { client, dialog } = renderDialog()
    fireEvent.change(await within(dialog).findByLabelText('账号名称'), { target: { value: '未提交' } })
    // fireEvent avoids the input blur that a normal mouse click would save first.
    fireEvent.click(within(dialog).getByRole('button', { name: '移除账号' }))
    const confirmation = await screen.findByRole('alertdialog')
    await waitFor(() => expect(within(confirmation).getByRole('button', { name: '移除账号' })).toBeEnabled())
    fireEvent.click(within(confirmation).getByRole('button', { name: '移除账号' }))
    expect(await within(dialog).findByRole('button', { name: '添加第一个账号' })).toBeInTheDocument()
    expect(client.updateProfile).not.toHaveBeenCalled()
  })

  it('keeps a sending failure in the confirmation and prevents duplicate removal or account switching while pending', async () => {
    const client = testClient([exampleAccount(), second])
    let reject!: (reason: unknown) => void
    client.removeAccount.mockReturnValue(new Promise((_, no) => { reject = no }))
    const { dialog } = renderDialog([exampleAccount(), second], {}, client)
    await userEvent.click(await within(dialog).findByRole('button', { name: '移除账号' }))
    const confirmation = await screen.findByRole('alertdialog')
    const remove = within(confirmation).getByRole('button', { name: '移除账号' })
    await waitFor(() => expect(remove).toBeEnabled())
    fireEvent.click(remove)
    fireEvent.click(remove)
    expect(client.removeAccount).toHaveBeenCalledOnce()
    expect(within(confirmation).getByRole('button', { name: '取消' })).toBeDisabled()
    expect(within(dialog).getByTitle('me@example.com · IMAP/SMTP')).toBeDisabled()
    await act(async () => reject(new MailxClientError({ code: 'alreadySending', message: 'private server reply', field: null })))
    expect(await within(confirmation).findByText('有邮件正在发送，请等待发送结束后再移除账号。')).toBeInTheDocument()
    expect(screen.queryByText('private server reply')).not.toBeInTheDocument()
    expect(within(confirmation).getByRole('button', { name: '移除账号' })).toBeEnabled()
  })

  it('keeps the active removal locked when another account is removed', async () => {
    const account = exampleAccount()
    const third = exampleAccount({ id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc', displayName: '备用邮箱', emailAddress: 'backup@example.com' })
    const client = testClient([account, second, third])
    const listeners = new Set<(removal: AccountRemoval) => void>()
    client.onAccountRemoved.mockImplementation((listener) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    })
    let reject!: (reason: unknown) => void
    client.removeAccount.mockReturnValue(new Promise((_, no) => { reject = no }))
    const { dialog, onClose, onAddAccount } = renderDialog([account, second, third], {}, client)
    const trigger = await within(dialog).findByRole('button', { name: '移除账号' })
    fireEvent.click(trigger)
    const confirmation = await screen.findByRole('alertdialog')
    const remove = within(confirmation).getByRole('button', { name: '移除账号' })
    await waitFor(() => expect(remove).toBeEnabled())
    fireEvent.click(remove)

    act(() => {
      for (const listener of [...listeners]) listener({ accountId: second.id, draftIds: [], credentialsPending: false })
    })
    act(() => {
      for (const listener of [...listeners]) listener({ accountId: second.id, draftIds: [], credentialsPending: false })
    })

    expect(screen.getByRole('alertdialog')).toBe(confirmation)
    expect(within(dialog).getByLabelText('账号名称')).toHaveValue('工作邮箱')
    expect(within(dialog).queryByTitle('me@example.com · IMAP/SMTP')).not.toBeInTheDocument()
    const otherAccount = within(dialog).getByTitle('backup@example.com · IMAP/SMTP')
    expect(otherAccount).toBeDisabled()
    fireEvent.click(otherAccount)
    const add = within(dialog).getByLabelText('添加账号')
    expect(add).toBeDisabled()
    fireEvent.click(add)
    fireEvent.keyDown(confirmation, { key: 'Escape' })
    expect(within(confirmation).getByRole('button', { name: '取消' })).toBeDisabled()
    expect(client.getAccount).not.toHaveBeenCalledWith(third.id)
    expect(onAddAccount).not.toHaveBeenCalled()
    expect(onClose).not.toHaveBeenCalled()

    await act(async () => reject(new MailxClientError({ code: 'alreadySending', message: 'private server reply', field: null })))
    expect(await within(confirmation).findByText('有邮件正在发送，请等待发送结束后再移除账号。')).toBeInTheDocument()
    expect(otherAccount).toBeEnabled()
    await userEvent.click(within(confirmation).getByRole('button', { name: '取消' }))
    await userEvent.click(otherAccount)
    await waitFor(() => expect(client.getAccount).toHaveBeenCalledWith(third.id))
    await waitFor(() => expect(within(dialog).getByLabelText('账号名称')).toHaveValue('备用邮箱'))

    act(() => {
      for (const listener of [...listeners]) listener({ accountId: third.id, draftIds: [], credentialsPending: false })
    })
    await waitFor(() => expect(within(dialog).getByLabelText('账号名称')).toHaveValue('工作邮箱'))
    act(() => {
      for (const listener of [...listeners]) listener({ accountId: third.id, draftIds: [], credentialsPending: false })
    })
    expect(within(dialog).getByLabelText('账号名称')).toHaveValue('工作邮箱')
    expect(within(dialog).queryByTitle('me@example.com · IMAP/SMTP')).not.toBeInTheDocument()

    act(() => {
      for (const listener of [...listeners]) listener({ accountId: account.id, draftIds: [], credentialsPending: false })
    })
    expect(await within(dialog).findByRole('button', { name: '添加第一个账号' })).toBeInTheDocument()
    act(() => {
      for (const listener of [...listeners]) listener({ accountId: account.id, draftIds: [], credentialsPending: false })
    })
    expect(within(dialog).getByRole('button', { name: '添加第一个账号' })).toBeInTheDocument()
    expect(within(dialog).queryByLabelText('账号名称')).not.toBeInTheDocument()
  })

  it('suppresses a late profile result after removal', async () => {
    const client = testClient([exampleAccount()])
    let resolve!: (value: Account) => void
    client.updateProfile.mockReturnValue(new Promise((yes) => { resolve = yes }))
    const { dialog, onAccountsChanged } = renderDialog([exampleAccount()], {}, client)
    fireEvent.change(await within(dialog).findByLabelText('账号名称'), { target: { value: '在途保存' } })
    fireEvent.blur(within(dialog).getByLabelText('账号名称'))
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledOnce())
    fireEvent.click(within(dialog).getByRole('button', { name: '移除账号' }))
    const confirmation = await screen.findByRole('alertdialog')
    await waitFor(() => expect(within(confirmation).getByRole('button', { name: '移除账号' })).toBeEnabled())
    fireEvent.click(within(confirmation).getByRole('button', { name: '移除账号' }))
    await within(dialog).findByRole('button', { name: '添加第一个账号' })
    const calls = onAccountsChanged.mock.calls.length
    await act(async () => resolve(exampleAccount({ displayName: '在途保存' })))
    expect(onAccountsChanged).toHaveBeenCalledTimes(calls)
    expect(within(dialog).queryByLabelText('账号名称')).not.toBeInTheDocument()
  })
})
