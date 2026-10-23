import { fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MailxClientError } from '@/lib/mailx-client'
import { exampleAccount, testClient } from './test-fixtures'
import { AddAccountDialog } from './add-account-dialog'

function renderDialog(client = testClient(), accounts = [exampleAccount({ color: '#2563eb' })]) {
  const onClose = vi.fn()
  const onCreated = vi.fn()
  render(<AddAccountDialog client={client} open accounts={accounts} onClose={onClose} onCreated={onCreated} />)
  return { client, onClose, onCreated, dialog: screen.getByRole('dialog', { name: '添加邮箱账号' }) }
}

function fillConnection(dialog: HTMLElement, password = 'synthetic-secret') {
  for (const [label, value] of Object.entries({
    '邮箱地址': 'new@example.com',
    'IMAP 主机': 'imap.example.com', 'SMTP 主机': 'smtp.example.com',
    'IMAP 登录用户名': 'new@example.com', 'SMTP 登录用户名': 'new@example.com',
    '收发密码': password,
  })) fireEvent.change(within(dialog).getByLabelText(label), { target: { value } })
}

async function chooseImap(dialog: HTMLElement) {
  await userEvent.click(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ }))
  await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
}

async function toProfile(dialog: HTMLElement) {
  await chooseImap(dialog)
  fillConnection(dialog)
  await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
  return within(dialog).findByText('最后，给这个邮箱起个名字')
}

describe('add account wizard', () => {
  it('starts with the method cards and shows no connection fields yet', () => {
    const { dialog } = renderDialog()
    expect(within(dialog).getByText('第 1 步，共 3 步')).toBeInTheDocument()
    expect(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ })).toBeInTheDocument()
    expect(within(dialog).queryByLabelText('邮箱地址')).not.toBeInTheDocument()
  })

  it('selects a provider on click and moves on only with the next button', async () => {
    const { dialog } = renderDialog()
    const next = within(dialog).getByRole('button', { name: '下一步' })
    expect(next).toBeDisabled()
    const card = within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ })
    expect(card).toHaveTextContent('其他邮箱')
    await userEvent.click(card)
    expect(card).toHaveAttribute('aria-checked', 'true')
    expect(within(dialog).queryByLabelText('邮箱地址')).not.toBeInTheDocument()
    await userEvent.click(next)
    expect(within(dialog).getByLabelText('邮箱地址')).toBeInTheDocument()
    expect(within(dialog).getByText('第 2 步，共 3 步')).toBeInTheDocument()
  })

  it('connects, then names the account with the address and a color no other account uses', async () => {
    const { client, dialog, onCreated } = renderDialog()
    await toProfile(dialog)
    expect(client.createAccount).not.toHaveBeenCalled()
    expect(within(dialog).getByText('第 3 步，共 3 步')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('账号名称')).toHaveValue('new@example.com')
    expect(within(dialog).getByRole('radio', { name: '蓝' })).toHaveAttribute('aria-checked', 'false')
    expect(within(dialog).getByRole('radio', { name: '绿' })).toHaveAttribute('aria-checked', 'true')

    fireEvent.change(within(dialog).getByLabelText('账号名称'), { target: { value: '新邮箱' } })
    await userEvent.click(within(dialog).getByRole('radio', { name: '紫' }))
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    const request = client.createAccount.mock.calls[0][0]
    expect(request.requestId).toMatch(/^[0-9a-f-]{36}$/)
    expect(request.config).toMatchObject({
      displayName: '新邮箱', color: '#7c3aed', emailAddress: 'new@example.com', providerId: null,
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-secret' } },
    })
  })

  it('sends the sender name with the new account, empty by default', async () => {
    const { client, dialog, onCreated } = renderDialog()
    await toProfile(dialog)
    const field = within(dialog).getByLabelText('发件人名称')
    expect(field).toHaveValue('')
    expect(field).toHaveAttribute('placeholder', '收件人看到的名字，可留空')
    fireEvent.change(field, { target: { value: ' 林薇 ' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config.senderName).toBe('林薇')
  })

  it('sends no sender name when the field is left empty', async () => {
    const { client, dialog, onCreated } = renderDialog()
    await toProfile(dialog)
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config.senderName).toBeNull()
  })

  it('shows a rejected sender name on the profile step', async () => {
    const client = testClient()
    client.createAccount.mockRejectedValue(new MailxClientError({ code: 'validation', field: 'senderName', message: '发件人名称过长。' }))
    const { dialog } = renderDialog(client)
    await toProfile(dialog)
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    expect(await within(dialog).findByText('发件人名称过长。')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('发件人名称')).toHaveAttribute('aria-invalid', 'true')
    expect(within(dialog).queryByLabelText('IMAP 主机')).not.toBeInTheDocument()
  })

  it('keeps the connection, password included, when going back from the last step', async () => {
    const { dialog } = renderDialog()
    await toProfile(dialog)
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    expect(within(dialog).getByLabelText('IMAP 主机')).toHaveValue('imap.example.com')
    expect(within(dialog).getByLabelText('收发密码')).toHaveValue('synthetic-secret')
  })

  it('returns to the connection step when the server rejects it', async () => {
    const client = testClient()
    client.createAccount.mockRejectedValue(new MailxClientError({ code: 'validation', field: 'imap.host', message: '无法使用这个主机。' }))
    const { dialog } = renderDialog(client)
    await toProfile(dialog)
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    expect(await within(dialog).findByText('无法使用这个主机。')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('IMAP 主机')).toHaveAttribute('aria-invalid', 'true')
    // The password went out with the failed request and must be entered again.
    expect(within(dialog).getByLabelText('收发密码')).toHaveValue('')
  })

  it('requires a name before finishing', async () => {
    const { client, dialog } = renderDialog()
    await toProfile(dialog)
    fireEvent.change(within(dialog).getByLabelText('账号名称'), { target: { value: '  ' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    expect(within(dialog).getByText('请填写账号名称。')).toBeInTheDocument()
    expect(client.createAccount).not.toHaveBeenCalled()
  })

  it('confirms before discarding a started connection', async () => {
    const { dialog, onClose } = renderDialog()
    await chooseImap(dialog)
    fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: 'draft@example.com' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    const confirm = await screen.findByRole('alertdialog')
    await userEvent.click(within(confirm).getByRole('button', { name: '继续编辑' }))
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveValue('draft@example.com')

    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' })
    await userEvent.click(await screen.findByRole('button', { name: '放弃修改' }))
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('goes back to the providers without a prompt when nothing was typed, keeping the choice', async () => {
    const { dialog } = renderDialog()
    await chooseImap(dialog)
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    expect(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ })).toHaveAttribute('aria-checked', 'true')
    expect(within(dialog).getByRole('button', { name: '下一步' })).toBeEnabled()
  })
})
