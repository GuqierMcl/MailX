import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { Account, AccountRemoval } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { exampleAccount, exampleSyncState, testClient } from './test-fixtures'
import { AccountsDialog } from './accounts-dialog'
import { i18n } from '@/i18n'
import { toast } from '@/components/ui/toast'

vi.mock('@/components/ui/toast', () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), close: vi.fn() } }))

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
    expect(within(dialog).getByText('连接方式')).toBeInTheDocument()
    expect(within(dialog).getAllByText('IMAP/SMTP').length).toBeGreaterThan(0)
    await userEvent.click(within(dialog).getByRole('button', { name: '连接详情' }))
    expect(within(dialog).getByText(/IMAP · imap\.example\.com:993 · TLS/)).toBeInTheDocument()
    expect(within(dialog).getByText('直接连接')).toBeInTheDocument()
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
    await userEvent.click(within(dialog).getByRole('button', { name: new RegExp('^' + other.displayName) }))
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

  it('reports a failed profile save with a toast', async () => {
    const client = testClient([exampleAccount()])
    client.updateProfile.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', field: null, message: '无法写入本机数据。' }))
    const { dialog } = renderDialog([exampleAccount()], {}, client)
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('radio', { name: '红' }))
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith('资料未保存', { description: '无法读写本地数据，请检查可用空间与访问权限。' }))
    // An event, not a state: nothing about it stays in the page.
    expect(within(dialog).queryByText('资料未保存')).not.toBeInTheDocument()
  })

  it('saves a name typed but not committed when switching accounts', async () => {
    const { client, dialog } = renderDialog([exampleAccount(), second])
    fireEvent.change(await within(dialog).findByLabelText('账号名称'), { target: { value: '没按回车' } })
    await userEvent.click(within(dialog).getByRole('button', { name: /^个人邮箱/ }))
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
    const pending = exampleAccount({ authentication: { kind: 'password', mode: 'shared', imap: { id: 'a', generation: 1, passwordConfigured: true, trust: 'needsConfirmation' }, smtp: { id: 'a', generation: 1, passwordConfigured: true, trust: 'needsConfirmation' } } })
    const { dialog, view } = renderDialog([pending])
    expect(await within(dialog).findByText('凭证需要重新确认')).toBeInTheDocument()
    view.unmount()

    const stopped = exampleSyncState({ phase: 'failed', failure: { code: 'authentication', message: '服务器拒绝了登录。', detail: null, willRetry: false } })
    const again = renderDialog([exampleAccount()], {}, testClient([exampleAccount()]), { [exampleAccount().id]: stopped })
    expect(await within(again.dialog).findByText('同步已停止')).toBeInTheDocument()
    expect(within(again.dialog).getByText(/邮箱服务器拒绝了账号登录/)).toBeInTheDocument()
  })

  it('keeps removing on the left of the fixed footer, and verifying and editing on the right', async () => {
    const { dialog } = renderDialog()
    const remove = await within(dialog).findByRole('button', { name: '移除账号' })
    const footer = dialog.querySelector('footer')!
    expect(remove.parentElement).toBe(footer)
    expect(remove).toHaveClass('text-destructive')
    const verify = within(dialog).getByRole('button', { name: '验证收发连接' })
    const edit = footer.querySelector('button:last-child')!
    expect(edit).toHaveTextContent('修改连接')
    expect(verify.parentElement).toBe(edit.parentElement)
    expect(verify.parentElement).not.toBe(footer)
    expect(footer.firstElementChild).toBe(remove)
    expect(footer.lastElementChild).toBe(verify.parentElement)
  })

  it('folds the connection details, and opens them with the reason under the failing server after a failed check', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockImplementation(async (accountId, revision) => ({
      accountId, revision,
      imap: { status: 'verified', stage: 'complete', code: null },
      smtp: { status: 'failed', stage: 'authentication', code: 'authentication' },
    }))
    const { dialog } = renderDialog([account], {}, client)
    const details = await within(dialog).findByRole('button', { name: '连接详情' })
    expect(details).toHaveAttribute('aria-expanded', 'false')
    await userEvent.click(within(dialog).getByRole('button', { name: '验证收发连接' }))
    expect(await within(dialog).findByText('· 发信未通过')).toHaveClass('text-destructive')
    expect(details).toHaveAttribute('aria-expanded', 'true')
    const smtpRow = within(dialog).getByText(/SMTP · smtp\.example\.com/).parentElement!.parentElement!
    expect(within(smtpRow).getByText(/邮箱服务器拒绝了账号登录/)).toHaveClass('text-xs', 'text-destructive')
    const imapRow = within(dialog).getByText(/IMAP · imap\.example\.com/).parentElement!.parentElement!
    expect(within(imapRow).getByText('已通过')).toBeInTheDocument()
    // The user can still fold it again.
    await userEvent.click(details)
    expect(details).toHaveAttribute('aria-expanded', 'false')
  })

  it('says a passed check does not test delivery', async () => {
    const { dialog } = renderDialog()
    expect(await within(dialog).findByText('验证已保存的连接和登录，不发送邮件。')).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('button', { name: '验证收发连接' }))
    expect(await within(dialog).findByText('· 收发均已通过')).toHaveClass('text-muted-foreground')
    expect(within(dialog).getByText('登录验证通过，尚未测试实际投递。')).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '连接详情' })).toHaveAttribute('aria-expanded', 'false')
  })

  it('shows a whole-request failure as a small line in the connection group, not a toast', async () => {
    const client = testClient([exampleAccount()])
    client.verifyAccountConnection.mockRejectedValue(new MailxClientError({ code: 'internal', field: null, message: 'x' }))
    const { dialog } = renderDialog([exampleAccount()], {}, client)
    await userEvent.click(await within(dialog).findByRole('button', { name: '验证收发连接' }))
    await waitFor(() => expect(dialog.querySelector('[role="status"].text-xs p.text-destructive')).not.toBeNull())
    expect(toast.error).not.toHaveBeenCalled()
  })

  it('shows credentials that need confirming or a missing password on the method row without opening the details', async () => {
    const pending = exampleAccount({ authentication: { kind: 'password', mode: 'shared', imap: { id: 'a', generation: 1, passwordConfigured: true, trust: 'needsConfirmation' }, smtp: { id: 'a', generation: 1, passwordConfigured: true, trust: 'needsConfirmation' } } })
    const { dialog, view } = renderDialog([pending])
    const details = await within(dialog).findByRole('button', { name: '连接详情' })
    expect(details).toHaveAttribute('aria-expanded', 'false')
    expect(within(dialog).getByText('· 凭证需要确认')).toHaveClass('text-destructive')
    view.unmount()

    const missing = exampleAccount({ authentication: { kind: 'password', mode: 'shared', imap: { id: 'a', generation: 1, passwordConfigured: false, trust: 'trusted' }, smtp: { id: 'a', generation: 1, passwordConfigured: false, trust: 'trusted' } } })
    const again = renderDialog([missing])
    expect(await within(again.dialog).findByText('· 缺少密码')).toHaveClass('text-destructive')
  })

  it('shows who the account is in the header, with the last sync time when healthy', async () => {
    const account = exampleAccount({ providerId: 'qq' })
    const { dialog } = renderDialog([account], {}, testClient([account]), { [account.id]: exampleSyncState() })
    await within(dialog).findByLabelText('账号名称')
    const header = dialog.querySelector('header')!
    expect(within(header).getByRole('heading', { name: '工作邮箱' })).toBeInTheDocument()
    expect(within(header).getByText('work@example.com')).toHaveClass('text-muted-foreground')
    expect(within(header).getByText(/^最近同步：/)).toHaveClass('text-muted-foreground')
    expect(header.querySelector('[data-slot="provider-badge"]')).not.toBeNull()
    // The header follows the name as it is typed.
    fireEvent.change(within(dialog).getByLabelText('账号名称'), { target: { value: '改名中' } })
    expect(within(header).getByRole('heading', { name: '改名中' })).toBeInTheDocument()
  })

  it('tells a failure that will retry from one that stopped, in the header', async () => {
    const account = exampleAccount()
    const failure = (willRetry: boolean) => ({ [account.id]: exampleSyncState({ phase: 'failed', failure: { code: 'network', message: '', detail: null, willRetry } }) })
    const retrying = renderDialog([account], {}, testClient([account]), failure(true))
    expect(await within(retrying.dialog).findByText(/^同步遇到问题，会自动重试：/)).not.toHaveClass('text-destructive')
    expect(within(retrying.dialog).queryByText('同步已停止')).not.toBeInTheDocument()
    expect(within(retrying.dialog).queryByText(/^最近同步：/)).not.toBeInTheDocument()
    expect(within(retrying.dialog).getAllByRole('button', { name: '修改连接' })).toHaveLength(1)
    retrying.view.unmount()

    const stopped = renderDialog([account], {}, testClient([account]), failure(false))
    expect(await within(stopped.dialog).findByText('同步已停止')).toHaveClass('text-destructive')
    expect(within(stopped.dialog).queryByText(/^同步遇到问题/)).not.toBeInTheDocument()
    // The repair action lives in the footer only.
    expect(within(stopped.dialog).getAllByRole('button', { name: '修改连接' })).toHaveLength(1)
  })

  it('lists each account with its address and marks the ones that have a problem', async () => {
    const failing = (willRetry: boolean) => exampleSyncState({ accountId: second.id, phase: 'failed', failure: { code: 'network', message: '', detail: null, willRetry } })
    const { dialog, view } = renderDialog([exampleAccount(), second], {}, testClient([exampleAccount(), second]), { [second.id]: failing(true) })
    const row = within(dialog).getByTitle('me@example.com · IMAP/SMTP')
    expect(within(row).getByText('me@example.com')).toHaveClass('opacity-70')
    expect(within(row).getByRole('img', { name: '同步遇到问题，会自动重试' })).toHaveTextContent('!')
    expect(within(row).getByRole('img', { name: '同步遇到问题，会自动重试' })).toHaveClass('text-amber-600')
    expect(within(within(dialog).getByTitle('work@example.com · IMAP/SMTP')).queryByRole('img')).not.toBeInTheDocument()
    view.unmount()

    const stopped = renderDialog([exampleAccount(), second], {}, testClient([exampleAccount(), second]), { [second.id]: failing(false) })
    expect(within(within(stopped.dialog).getByTitle('me@example.com · IMAP/SMTP')).getByRole('img', { name: '同步已停止' })).toHaveClass('text-destructive')
  })

  it('opens on the first account with a problem, stopped before retrying, unless a target is given', async () => {
    const third = exampleAccount({ id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc', displayName: '备用邮箱', emailAddress: 'backup@example.com' })
    const rows = [exampleAccount(), second, third]
    const states = {
      [exampleAccount().id]: exampleSyncState({ accountId: exampleAccount().id, phase: 'failed', failure: { code: 'network', message: '', detail: null, willRetry: true } }),
      [third.id]: exampleSyncState({ accountId: third.id, phase: 'failed', failure: { code: 'authentication', message: '', detail: null, willRetry: false } }),
    }
    const first = renderDialog(rows, {}, testClient(rows), states)
    await waitFor(() => expect(within(first.dialog).getByLabelText('账号名称')).toHaveValue('备用邮箱'))
    expect(first.client.getAccount).not.toHaveBeenCalledWith(exampleAccount().id)
    first.view.unmount()

    const explicit = renderDialog(rows, { accountId: second.id }, testClient(rows), states)
    await waitFor(() => expect(within(explicit.dialog).getByLabelText('账号名称')).toHaveValue('个人邮箱'))
    explicit.view.unmount()

    const healthy = renderDialog(rows)
    await waitFor(() => expect(within(healthy.dialog).getByLabelText('账号名称')).toHaveValue('工作邮箱'))
  })

  it('saves a custom color when the picker settles, previewing it before that, with readable avatar text', async () => {
    const { client, dialog } = renderDialog()
    await within(dialog).findByLabelText('账号名称')
    const picker = within(dialog).getByLabelText('自定义颜色')
    const avatar = () => dialog.querySelector('header [data-slot="account-avatar"]')!
    expect(avatar()).toHaveClass('bg-muted')
    fireEvent.input(picker, { target: { value: '#fde68a' } })
    expect(avatar()).toHaveStyle({ backgroundColor: '#fde68a', color: '#000000' })
    expect(picker.closest('label')).toHaveAttribute('data-checked')
    expect(client.updateProfile).not.toHaveBeenCalled()
    fireEvent.change(picker)
    await waitFor(() => expect(client.updateProfile).toHaveBeenCalledOnce())
    expect(client.updateProfile).toHaveBeenCalledWith({ accountId: exampleAccount().id, profile: { displayName: '工作邮箱', color: '#fde68a', senderName: null, useProviderIcon: true } })
    // A preset still works and takes the selection from the custom color.
    await userEvent.click(within(dialog).getByRole('radio', { name: '紫' }))
    expect(within(dialog).getByRole('radio', { name: '紫' })).toHaveAttribute('aria-checked', 'true')
    expect(picker.closest('label')).not.toHaveAttribute('data-checked')
    expect(avatar()).toHaveStyle({ color: '#ffffff' })
  })

  it('shows an empty state with an add button when there are no accounts', async () => {
    const { dialog, onAddAccount } = renderDialog([])
    await userEvent.click(within(dialog).getByRole('button', { name: '添加第一个账号' }))
    expect(onAddAccount).toHaveBeenCalledOnce()
  })

  it('confirms removal from the footer, and cancellation leaves the account intact', async () => {
    const { client, dialog } = renderDialog()
    const remove = await within(dialog).findByRole('button', { name: '移除账号' })
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
    expect(within(dialog).queryByRole('button', { name: /^工作邮箱/ })).not.toBeInTheDocument()
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
    const add = within(dialog).getByRole('button', { name: '添加账号', hidden: true })
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

  describe('an account that signs in through the browser', () => {
    const endpoint = { id: 'oauth-profile', generation: 1, credentialConfigured: true, trust: 'trusted' as const }
    const oauthAccount = (state: 'connected' | 'needsReauthorization' = 'connected', trust: 'trusted' | 'needsConfirmation' = 'trusted') => exampleAccount({
      providerId: 'microsoft-work', emailAddress: 'lin@contoso.com',
      imap: { host: 'outlook.office365.com', port: 993, tls: 'implicit', username: 'lin@contoso.com' },
      smtp: { host: 'smtp.office365.com', port: 587, tls: 'startTls', username: 'lin@contoso.com' },
      authentication: { kind: 'oauth2', configuration: { providerId: 'microsoft-work', issuer: 'https://issuer.example', clientRegistrationId: 'registration', authorizationSubject: 'subject', targetMailboxResource: null }, state, imap: { ...endpoint, trust }, smtp: { ...endpoint, trust } },
    })
    const failure = (code: 'signInRequired' | 'providerPolicy' | 'credentialMissing') => exampleSyncState({
      accountId: exampleAccount().id, phase: 'failed', failure: { code, message: '', detail: null, willRetry: false },
    })

    it('has no edit-connection button, but keeps removing and verifying, and shows the real servers', async () => {
      const account = oauthAccount()
      const { dialog } = renderDialog([account])
      expect(await within(dialog).findByText('Microsoft 365 / Exchange · 浏览器登录')).toBeInTheDocument()
      await userEvent.click(within(dialog).getByRole('button', { name: '连接详情' }))
      expect(await within(dialog).findByText(/IMAP · outlook\.office365\.com:993 · TLS · lin@contoso\.com/)).toBeInTheDocument()
      expect(within(dialog).getByText(/SMTP · smtp\.office365\.com:587 · STARTTLS · lin@contoso\.com/)).toBeInTheDocument()
      expect(within(dialog).queryByRole('button', { name: '修改连接' })).not.toBeInTheDocument()
      expect(within(dialog).getByRole('button', { name: '移除账号' })).toBeInTheDocument()
      expect(within(dialog).getByRole('button', { name: '验证收发连接' })).toBeInTheDocument()
      expect(within(dialog).getByRole('button', { name: '重新登录' })).toBeInTheDocument()
    })

    it('puts the sign-in renewal in the footer, and only a needed one in the header', async () => {
      const { dialog, view } = renderDialog([oauthAccount()])
      const remove = await within(dialog).findByRole('button', { name: '移除账号' })
      const signIn = within(dialog).getByRole('button', { name: '重新登录' })
      // Beside verifying on the right of the footer; removing stays on the left.
      const footer = dialog.querySelector('footer')!
      expect(footer.contains(signIn) && footer.contains(remove)).toBe(true)
      expect(signIn.parentElement).toBe(within(dialog).getByRole('button', { name: '验证收发连接' }).parentElement)
      expect(remove.parentElement).toBe(footer)
      expect(within(dialog).queryByText('需要重新登录，这个邮箱才能继续使用')).not.toBeInTheDocument()
      view.unmount()

      const second = renderDialog([oauthAccount('needsReauthorization')])
      const warning = await within(second.dialog).findByText('需要重新登录，这个邮箱才能继续使用')
      const buttons = within(second.dialog).getAllByRole('button', { name: '重新登录' })
      expect(buttons).toHaveLength(1)
      expect(second.dialog.querySelector('footer')).toContainElement(buttons[0])
      expect(second.dialog.querySelector('header')).toContainElement(warning)
    })

    it('offers signing in again instead of editing when the sign-in is no longer valid', async () => {
      const account = oauthAccount()
      for (const code of ['signInRequired', 'credentialMissing'] as const) {
        const { dialog, view } = renderDialog([account], {}, testClient([account]), { [account.id]: { ...failure(code), accountId: account.id } })
        expect(await within(dialog).findByText('需要重新登录，这个邮箱才能继续使用')).toBeInTheDocument()
        expect(within(dialog).getByRole('button', { name: '重新登录' })).toBeInTheDocument()
        expect(within(dialog).queryByText('同步已停止')).not.toBeInTheDocument()
        expect(within(dialog).queryByRole('button', { name: '修改连接' })).not.toBeInTheDocument()
        view.unmount()
      }
    })

    it('shows other stopped-sync reasons without an edit button or a promise to retry after editing', async () => {
      const account = oauthAccount()
      const { dialog } = renderDialog([account], {}, testClient([account]), { [account.id]: { ...failure('providerPolicy'), accountId: account.id } })
      expect(await within(dialog).findByText('同步已停止')).toBeInTheDocument()
      expect(within(dialog).queryByRole('button', { name: '修改连接' })).not.toBeInTheDocument()
      expect(within(dialog).queryByText('修改连接后会重新尝试。')).not.toBeInTheDocument()
    })

    it('does not tell the user to edit the connection when the saved servers no longer match', async () => {
      const account = oauthAccount('connected', 'needsConfirmation')
      const { dialog } = renderDialog([account])
      expect(await within(dialog).findByText('凭证需要重新确认')).toBeInTheDocument()
      expect(within(dialog).getByText(/请移除该账号后重新添加/)).toBeInTheDocument()
      expect(within(dialog).queryByRole('button', { name: '修改连接' })).not.toBeInTheDocument()
    })

    it('leaves password accounts unchanged', async () => {
      const { dialog } = renderDialog([exampleAccount()], {}, testClient([exampleAccount()]), { [exampleAccount().id]: exampleSyncState({ phase: 'failed', failure: { code: 'authentication', message: '', detail: null, willRetry: false } }) })
      expect(await within(dialog).findByText('同步已停止')).toBeInTheDocument()
      expect(within(dialog).getAllByRole('button', { name: '修改连接' })).toHaveLength(1)
      expect(within(dialog).queryByRole('button', { name: '重新登录' })).not.toBeInTheDocument()
    })
  })
})
