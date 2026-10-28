import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { Account, ConnectionInput } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import type { AccountDraft } from './account-draft'
import { ConnectionForm } from './connection-form'
import { exampleAccount } from './test-fixtures'
import { NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER, QQ_PROVIDER, type MailProvider } from './providers'

type Submit = (connection: ConnectionInput, draft: AccountDraft) => Promise<void> | void

function renderForm(account: Account | null = null, onSubmit: Submit = vi.fn(), provider?: MailProvider) {
  const submit = vi.fn(onSubmit)
  const onReload = vi.fn()
  render(
    <ConnectionForm account={account} provider={provider} intro={{ title: '说明', description: '说明' }} submitLabel={account ? '保存更改' : '保存账号'}
      submittingLabel="保存中…" onSubmit={submit} onReload={onReload} onBusyChange={vi.fn()} onDirtyChange={vi.fn()} />,
  )
  return { submit, onReload }
}

function fillBasic() {
  for (const [label, value] of Object.entries({
    '邮箱地址': 'me@example.com',
    'IMAP 主机': 'imap.example.com', 'SMTP 主机': 'smtp.example.com',
    'IMAP 登录用户名': 'receive@example.com', 'SMTP 登录用户名': 'send@example.com',
  })) fireEvent.change(screen.getByLabelText(label), { target: { value } })
}

describe('connection form', () => {
  it('asks only for the connection, not the profile', () => {
    renderForm()
    expect(screen.queryByLabelText('账号名称')).not.toBeInTheDocument()
    expect(screen.getByLabelText('邮箱地址')).toBeInTheDocument()
  })

  it('keeps usernames empty while previewing and submitting the current email as their default', async () => {
    const { submit } = renderForm()
    const imapUsername = screen.getByLabelText('IMAP 登录用户名')
    const smtpUsername = screen.getByLabelText('SMTP 登录用户名')
    expect(imapUsername).toHaveAttribute('placeholder', '默认使用邮箱地址')
    expect(imapUsername).toHaveAccessibleDescription('留空时使用邮箱地址。')
    for (const [label, value] of Object.entries({
      '邮箱地址': ' first@example.com ', 'IMAP 主机': 'imap.example.com', 'SMTP 主机': 'smtp.example.com',
      '收发密码': 'synthetic-secret',
    })) fireEvent.change(screen.getByLabelText(label), { target: { value } })
    expect(imapUsername).toHaveAttribute('placeholder', 'first@example.com')
    fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: 'current@example.com' } })
    expect(imapUsername).toHaveAttribute('placeholder', 'current@example.com')
    expect(smtpUsername).toHaveAttribute('placeholder', 'current@example.com')
    expect(imapUsername).toHaveValue('')
    expect(smtpUsername).toHaveValue('')
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    const [connection, draft] = submit.mock.calls[0]
    expect(connection).toMatchObject({ imap: { username: 'current@example.com' }, smtp: { username: 'current@example.com' } })
    expect(draft.imap.username).toBe('')
    expect(draft.smtp.username).toBe('')
  })

  it('shows saved usernames on edit and uses the new email only after a username is cleared', async () => {
    const account = exampleAccount()
    const { submit } = renderForm(account)
    expect(screen.getByLabelText('IMAP 登录用户名')).toHaveValue(account.imap.username)
    expect(screen.getByLabelText('SMTP 登录用户名')).toHaveValue(account.smtp.username)
    fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: 'new@example.com' } })
    fireEvent.change(screen.getByLabelText('SMTP 登录用户名'), { target: { value: '' } })
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    expect(submit.mock.calls[0][0]).toMatchObject({
      imap: { username: account.imap.username }, smtp: { username: 'new@example.com' },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } },
    })
  })

  it.each([QQ_PROVIDER, NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER])('keeps saved $id endpoints, login identities, credentials and proxy instead of reapplying its preset', async (provider) => {
    const account = exampleAccount({
      providerId: provider.id, emailAddress: `synthetic@${provider.preset!.domains[0]}`,
      imap: { host: 'imap.custom.example', port: 1993, tls: 'implicit', username: 'receive-identity@qq.com' },
      smtp: { host: 'smtp.custom.example', port: 1587, tls: 'startTls', username: 'send-identity@qq.com' },
      route: { kind: 'custom', proxyKind: 'httpConnect', host: 'proxy.example.com', port: 8080, dns: 'proxy', imageProxyDns: false, authentication: null },
    })
    const { submit } = renderForm(account, vi.fn(), provider)
    const advanced = screen.getByText('高级设置').closest('details')
    if (!advanced?.open) await userEvent.click(screen.getByText('高级设置'))
    expect(screen.getByLabelText('IMAP 主机')).toHaveValue(account.imap.host)
    expect(screen.getByLabelText('SMTP 主机')).toHaveValue(account.smtp.host)
    expect(screen.getByLabelText('IMAP 登录用户名')).toHaveValue(account.imap.username)
    expect(screen.getByLabelText('SMTP 登录用户名')).toHaveValue(account.smtp.username)
    expect(screen.getByLabelText('代理主机')).toHaveValue('proxy.example.com')
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    expect(submit).toHaveBeenCalledOnce()
    expect(submit.mock.calls[0][0]).toMatchObject({
      providerId: provider.id, emailAddress: account.emailAddress,
      imap: account.imap, smtp: account.smtp, route: account.route,
      authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } },
    })
  })

  it.each([QQ_PROVIDER, NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER])('opens $id advanced settings and focuses a hidden field rejected by the backend', async (provider) => {
    const account = exampleAccount({ providerId: provider.id, emailAddress: `synthetic@${provider.preset!.domains[0]}` })
    renderForm(account, () => {
      throw new MailxClientError({ code: 'validation', field: 'imap.host', message: '服务器无效。' })
    }, provider)
    const summary = screen.getByText('高级设置')
    const details = summary.closest('details')
    expect(details).toHaveAttribute('open')
    await userEvent.click(summary)
    expect(details).not.toHaveAttribute('open')
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    const host = screen.getByLabelText('IMAP 主机')
    await waitFor(() => {
      expect(details).toHaveAttribute('open')
      expect(host).toBeVisible()
      expect(host).toHaveAttribute('aria-invalid', 'true')
      expect(host).toHaveFocus()
    })
  })

  it('keeps the username default hint accessible alongside a field error', async () => {
    renderForm(exampleAccount(), () => {
      throw new MailxClientError({ code: 'validation', field: 'imap.username', message: '登录用户名无效。' })
    })
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    const username = screen.getByLabelText('IMAP 登录用户名')
    expect(username).toHaveAttribute('aria-invalid', 'true')
    expect(username).toHaveAttribute('aria-describedby', 'imap.username-description imap.username-error')
    expect(username).toHaveAccessibleDescription(/留空时使用邮箱地址。/)
    expect(document.getElementById('imap.username-error')).toBeInTheDocument()
  })

  it('submits shared credentials once, clears secret input while pending, and retains different login names', async () => {
    let finish!: () => void
    const { submit } = renderForm(null, () => new Promise<void>((resolve) => { finish = resolve }))
    fillBasic()
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'synthetic-shared-secret' } })
    const save = screen.getByRole('button', { name: '保存账号' })
    fireEvent.click(save)
    fireEvent.click(save)
    expect(submit).toHaveBeenCalledTimes(1)
    expect(screen.getByLabelText('收发密码')).toHaveValue('')
    expect(screen.getByRole('button', { name: '保存中…' })).toBeDisabled()
    const [connection, draft] = submit.mock.calls[0]
    expect(connection).not.toHaveProperty('displayName')
    expect(connection.authentication).toEqual({ kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-shared-secret' } })
    expect(connection.imap.username).toBe('receive@example.com')
    expect(connection.smtp.username).toBe('send@example.com')
    // The caller gets the draft as submitted, so a wizard can come back to it.
    expect(draft.sharedPassword.value).toBe('synthetic-shared-secret')
    await act(async () => { finish() })
    expect(screen.getByRole('button', { name: '保存账号' })).toBeEnabled()
  })

  it('switches to separate credentials without retaining the hidden shared secret', async () => {
    const user = userEvent.setup()
    const { submit } = renderForm()
    fillBasic()
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'must-be-cleared' } })
    await user.click(screen.getByRole('radio', { name: '收发分别设置密码' }))
    expect(screen.queryByLabelText('收发密码')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('收信密码'), { target: { value: 'receive-secret' } })
    fireEvent.change(screen.getByLabelText('发信密码'), { target: { value: 'send-secret' } })
    await user.click(screen.getByRole('button', { name: '保存账号' }))
    expect(submit.mock.calls[0][0].authentication).toEqual({ kind: 'password', mode: 'separate', imapPassword: { action: 'replace', value: 'receive-secret' }, smtpPassword: { action: 'replace', value: 'send-secret' } })
  })

  it.each([NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER])('clears an in-flight $id authorization code and prevents duplicate submissions', async (provider) => {
    let finish!: () => void
    const { submit } = renderForm(null, () => new Promise<void>((resolve) => { finish = resolve }), provider)
    fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: `synthetic@${provider.preset!.domains[0]}` } })
    fireEvent.change(screen.getByLabelText('授权码'), { target: { value: 'synthetic-authorization-code' } })
    const save = screen.getByRole('button', { name: '保存账号' })
    fireEvent.click(save)
    fireEvent.click(save)
    expect(submit).toHaveBeenCalledOnce()
    expect(screen.getByLabelText('授权码')).toHaveValue('')
    expect(screen.getByRole('button', { name: '保存中…' })).toBeDisabled()
    expect(submit.mock.calls[0][0].authentication).toEqual({ kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-authorization-code' } })
    await act(async () => { finish() })
    expect(screen.getByRole('button', { name: '保存账号' })).toBeEnabled()
  })

  it('edits with explicit keep actions without reading any password', async () => {
    const { submit } = renderForm(exampleAccount())
    expect(screen.queryByLabelText('收发密码')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    expect(submit.mock.calls[0][0]).toMatchObject({ emailAddress: 'work@example.com', authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } } })
  })

  it('shows conflict, clears a replacement password, and preserves non-sensitive edits', async () => {
    const user = userEvent.setup()
    const { onReload } = renderForm(exampleAccount(), () => {
      throw new MailxClientError({ code: 'conflict', field: null, message: '配置已被修改，请重新加载后重试。' })
    })
    fireEvent.change(screen.getByLabelText('IMAP 主机'), { target: { value: 'imap.changed.example' } })
    await user.click(screen.getByRole('switch', { name: '替换收发密码' }))
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'new-synthetic-secret' } })
    await user.click(screen.getByRole('button', { name: '保存更改' }))
    expect(await screen.findByText('配置已在其他位置更新')).toBeInTheDocument()
    expect(screen.getByLabelText('收发密码')).toHaveValue('')
    expect(screen.getByLabelText('IMAP 主机')).toHaveValue('imap.changed.example')
    await user.click(screen.getByRole('button', { name: '重新加载配置' }))
    expect(onReload).toHaveBeenCalledOnce()
  })

  it('does not submit invalid ports and clears secrets after validation failure', async () => {
    const { submit } = renderForm()
    fillBasic()
    fireEvent.change(screen.getByLabelText('IMAP 端口'), { target: { value: '0' } })
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'synthetic-secret' } })
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    expect(submit).not.toHaveBeenCalled()
    expect(screen.getByLabelText('IMAP 端口')).toHaveAttribute('aria-invalid', 'true')
    expect(screen.getByLabelText('收发密码')).toHaveValue('')
    expect(screen.getByLabelText('邮箱地址')).toHaveValue('me@example.com')
  })

  it('shows a credential-store failure without claiming success', async () => {
    renderForm(exampleAccount(), () => {
      throw new MailxClientError({ code: 'secretStoreLocked', message: '系统凭据库已锁定，请解锁后重试。', field: null })
    })
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    await waitFor(() => expect(screen.getByText(/系统凭据库已锁定/)).toBeInTheDocument())
    expect(screen.getByRole('button', { name: '保存更改' })).toBeEnabled()
  })

  it('starts from a resumed draft and shows an error passed in', () => {
    render(
      <ConnectionForm account={null} intro={{ title: '说明', description: '说明' }} submitLabel="下一步" submittingLabel="下一步"
        initialError={{ code: 'validation', field: 'imap.host', message: '无法使用这个主机。' }}
        initialDraft={{ ...(() => { const d = { emailAddress: 'me@example.com' }; return d })(), imap: { host: 'bad', port: '993', tls: 'implicit', username: 'me' }, smtp: { host: 'smtp.example.com', port: '465', tls: 'implicit', username: 'me' }, mode: 'shared', sharedPassword: { replace: true, value: '' }, imapPassword: { replace: true, value: '' }, smtpPassword: { replace: true, value: '' }, routeKind: 'direct', proxyKind: 'socks5', proxyHost: '', proxyPort: '1080', proxyAuth: false, proxyUsername: '', proxyPassword: { replace: true, value: '' }, imageProxyDns: false, confirmImageProxyDns: false }}
        onSubmit={vi.fn()} onBusyChange={vi.fn()} onDirtyChange={vi.fn()} />,
    )
    expect(screen.getByLabelText('邮箱地址')).toHaveValue('me@example.com')
    expect(screen.getByLabelText('IMAP 主机')).toHaveAttribute('aria-invalid', 'true')
    expect(screen.getByText('请填写主机名或 IP 地址，不要包含协议、路径、用户名或密码。')).toBeInTheDocument()
  })

  it('revokes the image proxy switch on endpoint edits and allows an explicit new selection', async () => {
    const account = exampleAccount({ route: { kind: 'custom', proxyKind: 'socks5', host: 'proxy.example.com', port: 1080, dns: 'proxy', imageProxyDns: true, authentication: null } })
    const { submit } = renderForm(account)
    const toggle = screen.getByRole('switch', { name: '通过此代理加载 HTTPS 图片' })
    expect(toggle).toBeChecked()
    fireEvent.change(screen.getByLabelText('代理主机'), { target: { value: 'other.example.com' } })
    expect(toggle).not.toBeChecked()
    await userEvent.click(toggle)
    await userEvent.click(screen.getByRole('button', { name: '保存更改' }))
    expect(submit.mock.calls[0][0].route).toMatchObject({ host: 'other.example.com', imageProxyDns: true, confirmImageProxyDns: true })
  })
})
