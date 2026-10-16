import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { Account } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { AccountForm } from './account-form'
import { exampleAccount, testClient } from './test-fixtures'

function renderForm(account: Account | null = null, client = testClient()) {
  const onSaved = vi.fn()
  const onReload = vi.fn()
  render(<AccountForm account={account} client={client} onSaved={onSaved} onReload={onReload} onBusyChange={vi.fn()} onDirtyChange={vi.fn()} />)
  return { client, onSaved, onReload }
}

function fillBasic() {
  for (const [label, value] of Object.entries({
    '账号名称': '个人邮箱', '邮箱地址': 'me@example.com',
    'IMAP 主机': 'imap.example.com', 'SMTP 主机': 'smtp.example.com',
    'IMAP 登录用户名': 'receive@example.com', 'SMTP 登录用户名': 'send@example.com',
  })) fireEvent.change(screen.getByLabelText(label), { target: { value } })
}

describe('account configuration form', () => {
  it('submits shared credentials once, clears secret input while pending, and retains different login names', async () => {
    const client = testClient()
    let finish!: (account: Account) => void
    client.createAccount.mockImplementation(() => new Promise((resolve) => { finish = resolve }))
    const { onSaved } = renderForm(null, client)
    fillBasic()
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'synthetic-shared-secret' } })
    const save = screen.getByRole('button', { name: '保存账号' })
    fireEvent.click(save)
    fireEvent.click(save)
    expect(client.createAccount).toHaveBeenCalledTimes(1)
    expect(screen.getByLabelText('收发密码')).toHaveValue('')
    expect(screen.getByRole('button', { name: '保存中…' })).toBeDisabled()
    const request = client.createAccount.mock.calls[0][0]
    expect(request.requestId).toMatch(/^[0-9a-f-]{36}$/)
    expect(request.config.authentication).toEqual({ kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-shared-secret' } })
    expect(request.config.imap.username).toBe('receive@example.com')
    expect(request.config.smtp.username).toBe('send@example.com')
    await act(async () => { finish(exampleAccount()) })
    expect(onSaved).toHaveBeenCalledOnce()
  })

  it('switches to separate credentials without retaining the hidden shared secret', async () => {
    const user = userEvent.setup()
    const { client } = renderForm()
    fillBasic()
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'must-be-cleared' } })
    await user.click(screen.getByRole('radio', { name: '收发分别设置密码' }))
    expect(screen.queryByLabelText('收发密码')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('收信密码'), { target: { value: 'receive-secret' } })
    fireEvent.change(screen.getByLabelText('发信密码'), { target: { value: 'send-secret' } })
    await user.click(screen.getByRole('button', { name: '保存账号' }))
    expect(client.createAccount.mock.calls[0][0].config.authentication).toEqual({ kind: 'password', mode: 'separate', imapPassword: { action: 'replace', value: 'receive-secret' }, smtpPassword: { action: 'replace', value: 'send-secret' } })
  })

  it('edits with explicit keep actions and the loaded revision without reading any password', async () => {
    const { client } = renderForm(exampleAccount())
    expect(screen.queryByLabelText('收发密码')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    expect(client.updateAccount.mock.calls[0][0]).toMatchObject({ accountId: exampleAccount().id, expectedRevision: 3, config: { authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } } } })
  })

  it('shows conflict, clears a replacement password, and preserves non-sensitive edits', async () => {
    const user = userEvent.setup()
    const client = testClient()
    client.updateAccount.mockRejectedValue(new MailxClientError({ code: 'conflict', field: null, message: '配置已被修改，请重新加载后重试。' }))
    const { onSaved, onReload } = renderForm(exampleAccount(), client)
    fireEvent.change(screen.getByLabelText('账号名称'), { target: { value: '新的名称' } })
    await user.click(screen.getByRole('switch', { name: '替换收发密码' }))
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'new-synthetic-secret' } })
    await user.click(screen.getByRole('button', { name: '保存更改' }))
    expect(await screen.findByText('配置已在其他位置更新')).toBeInTheDocument()
    expect(screen.getByLabelText('收发密码')).toHaveValue('')
    expect(screen.getByLabelText('账号名称')).toHaveValue('新的名称')
    expect(onSaved).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: '重新加载配置' }))
    expect(onReload).toHaveBeenCalledOnce()
  })

  it('does not save invalid ports and clears secrets after validation failure', async () => {
    const { client } = renderForm()
    fillBasic()
    fireEvent.change(screen.getByLabelText('IMAP 端口'), { target: { value: '0' } })
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'synthetic-secret' } })
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    expect(client.createAccount).not.toHaveBeenCalled()
    expect(screen.getByLabelText('IMAP 端口')).toHaveAttribute('aria-invalid', 'true')
    expect(screen.getByLabelText('收发密码')).toHaveValue('')
    expect(screen.getByLabelText('邮箱地址')).toHaveValue('me@example.com')
  })

  it('shows a credential-store failure without claiming a successful save', async () => {
    const client = testClient()
    client.updateAccount.mockRejectedValue(new MailxClientError({ code: 'secretStoreLocked', message: '系统凭据库已锁定，请解锁后重试。', field: null }))
    const { onSaved } = renderForm(exampleAccount(), client)
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    await waitFor(() => expect(screen.getByText(/系统凭据库已锁定/)).toBeInTheDocument())
    expect(onSaved).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: '保存更改' })).toBeEnabled()
  })
})
