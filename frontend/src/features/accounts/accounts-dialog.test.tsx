import { fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { Account } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { exampleAccount, exampleSyncState, testClient } from './test-fixtures'
import { AccountsDialog } from './accounts-dialog'

const second = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人邮箱', emailAddress: 'me@example.com', color: '#16a34a' })

function renderDialog(rows: Account[] = [exampleAccount()], target: { accountId?: string } = {}, client = testClient(rows), syncStates = {}) {
  const onClose = vi.fn()
  const onAccountsChanged = vi.fn()
  const onAddAccount = vi.fn()
  const view = render(<AccountsDialog client={client} target={target} accounts={rows} syncStates={syncStates} onClose={onClose} onAccountsChanged={onAccountsChanged} onAddAccount={onAddAccount} />)
  return { client, onClose, onAccountsChanged, onAddAccount, view, dialog: screen.getByRole('dialog', { name: '邮箱账号' }) }
}

describe('accounts dialog', () => {
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
    expect(client.updateProfile).toHaveBeenCalledWith({ accountId: exampleAccount().id, profile: { displayName: '新名称', color: null, senderName: null } })
    expect(client.updateConnection).not.toHaveBeenCalled()
    expect(await within(dialog).findByText('已保存')).toBeInTheDocument()
  })

  it('saves a color as soon as it is picked, keeping the saved name', async () => {
    const { client, dialog } = renderDialog()
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('radio', { name: '紫' }))
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledWith({ accountId: exampleAccount().id, profile: { displayName: '工作邮箱', color: '#7c3aed', senderName: null } }))
    expect(within(dialog).getByRole('radio', { name: '紫' })).toHaveAttribute('aria-checked', 'true')
  })

  it('saves the sender name when editing settles, trimmed, and clears it with an empty value', async () => {
    const { client, dialog } = renderDialog([exampleAccount({ senderName: '林薇' })])
    const field = await within(dialog).findByLabelText('发件人名称')
    expect(field).toHaveValue('林薇')
    expect(field).toHaveAttribute('placeholder', '收件人看到的名字，可留空')
    fireEvent.change(field, { target: { value: '  Lin Wei ' } })
    fireEvent.blur(field)
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledWith({
      accountId: exampleAccount().id, profile: { displayName: '工作邮箱', color: null, senderName: 'Lin Wei' },
    }))
    expect(field).toHaveValue('Lin Wei')

    fireEvent.change(field, { target: { value: ' ' } })
    fireEvent.keyDown(field, { key: 'Enter' })
    await waitFor(() => expect(client.updateProfile).toHaveBeenLastCalledWith({
      accountId: exampleAccount().id, profile: { displayName: '工作邮箱', color: null, senderName: null },
    }))
  })

  it('shows a rejected sender name on its own field', async () => {
    const client = testClient()
    client.updateProfile.mockRejectedValue(new MailxClientError({ code: 'validation', field: 'senderName', message: '发件人名称不能包含控制字符。' }))
    const { dialog } = renderDialog([exampleAccount()], {}, client)
    const field = await within(dialog).findByLabelText('发件人名称')
    fireEvent.change(field, { target: { value: 'bad' } })
    fireEvent.blur(field)
    expect(await within(dialog).findByText('发件人名称不能包含控制字符。')).toBeInTheDocument()
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
    expect(within(dialog).getByText('无法写入本机数据。')).toBeInTheDocument()
  })

  it('saves a name typed but not committed when switching accounts', async () => {
    const { client, dialog } = renderDialog([exampleAccount(), second])
    fireEvent.change(await within(dialog).findByLabelText('账号名称'), { target: { value: '没按回车' } })
    await userEvent.click(within(dialog).getByRole('button', { name: /个人邮箱/ }))
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledWith({ accountId: exampleAccount().id, profile: { displayName: '没按回车', color: null, senderName: null } }))
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
    expect(within(again.dialog).getByText(/服务器拒绝了登录/)).toBeInTheDocument()
  })

  it('shows an empty state with an add button when there are no accounts', async () => {
    const { dialog, onAddAccount } = renderDialog([])
    await userEvent.click(within(dialog).getByRole('button', { name: '添加第一个账号' }))
    expect(onAddAccount).toHaveBeenCalledOnce()
  })
})
