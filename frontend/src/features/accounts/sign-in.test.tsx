import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { MailSettingsDiscovery } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { AddAccountDialog } from './add-account-dialog'
import { ConnectionForm } from './connection-form'
import { exampleAccount, testClient } from './test-fixtures'

const EWS_URL = 'https://mail.example.com/EWS/Exchange.asmx'
const WAIT = { timeout: 3000 }

type Report = Awaited<ReturnType<MailxClient['verifyDraftConnection']>>
const success: Report = {
  verificationId: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
  imap: { status: 'verified', stage: 'complete', code: null },
  smtp: { status: 'verified', stage: 'complete', code: null },
}

/** What "sign in" hopes for: an Exchange endpoint recommended by the server. */
function ewsDiscovery(): MailSettingsDiscovery {
  return {
    status: 'found',
    ews: { url: EWS_URL, auth: 'ntlm', username: null, certPrincipalName: null },
    imap: null, smtp: null, source: 'https://mail.example.com',
  }
}

function renderSignInForm(client = testClient(), onSubmit = vi.fn()) {
  render(
    <ConnectionForm account={null} discoveryClient={client} verificationClient={client} signInFirst
      intro={{ title: '说明', description: '说明' }} submitLabel="保存账号" submittingLabel="保存中…"
      onSubmit={onSubmit} onBusyChange={vi.fn()} onDirtyChange={vi.fn()} />,
  )
  return { client, onSubmit }
}

function fillSignIn(emailAddress = 'alice@example.com', password = 'synthetic-secret') {
  fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: emailAddress } })
  fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: password } })
}

describe('signing in before the manual server form', () => {
  it('opens a generic mailbox on the sign-in card with no server fields', async () => {
    const client = testClient()
    render(<AddAccountDialog client={client} open accounts={[]} onClose={vi.fn()} onCreated={vi.fn()} />)
    const dialog = screen.getByRole('dialog', { name: '添加邮箱账号' })
    await userEvent.click(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ }))
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(within(dialog).getByText('登录邮箱')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('邮箱地址')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('收发密码')).toBeInTheDocument()
    // The card's primary action is the wizard's own submit label.
    expect(within(dialog).getByRole('button', { name: '下一步' })).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '手工设置' })).toBeInTheDocument()
    expect(within(dialog).queryByLabelText('IMAP 主机')).not.toBeInTheDocument()
    expect(within(dialog).queryByLabelText('SMTP 主机')).not.toBeInTheDocument()
  })

  it('signs in with the detected Exchange endpoint and submits verified placeholder rows', async () => {
    const client = testClient()
    client.discoverMailSettings.mockResolvedValue(ewsDiscovery())
    const { onSubmit } = renderSignInForm(client)
    fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: 'alice@example.com' } })
    // The card's own address lookup lands first, as it does while the user types the password.
    await waitFor(() => expect(client.discoverMailSettings).toHaveBeenCalledOnce(), WAIT)
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 100)) })
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'synthetic-secret' } })
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce(), WAIT)
    expect(client.discoverMailSettings).toHaveBeenCalledWith({
      emailAddress: 'alice@example.com', password: 'synthetic-secret', route: { kind: 'direct' },
    })
    expect(client.verifyDraftConnection).toHaveBeenCalledOnce()
    const [connection] = onSubmit.mock.calls[0]
    expect(connection).toMatchObject({
      emailAddress: 'alice@example.com',
      ews: { url: EWS_URL, auth: 'ntlm', username: 'alice@example.com', certPrincipalName: null },
      imap: { host: 'mail.example.com', port: 443, tls: 'implicit', username: 'alice@example.com' },
      smtp: { host: 'mail.example.com', port: 443, tls: 'implicit', username: 'alice@example.com' },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-secret' } },
      route: { kind: 'direct' },
    })
  })

  it('falls back to the prefilled Exchange form with the report when verification fails', async () => {
    const client = testClient()
    client.discoverMailSettings.mockResolvedValue(ewsDiscovery())
    client.verifyDraftConnection.mockResolvedValue({ ...success, imap: { status: 'failed', stage: 'authentication', code: 'authentication' } })
    const { onSubmit } = renderSignInForm(client)
    fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: 'alice@example.com' } })
    // The card's own address lookup lands first, as it does while the user types the password.
    await waitFor(() => expect(client.discoverMailSettings).toHaveBeenCalledOnce(), WAIT)
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 100)) })
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'synthetic-secret' } })
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    // The failed sign-in hands the prefilled form over for manual correction.
    expect(await screen.findByLabelText('Exchange 服务器 URL')).toHaveValue(EWS_URL)
    expect(screen.getByRole('button', { name: '测试登录' })).toBeInTheDocument()
    // The redesigned report shows a per-protocol verdict instead of a banner.
    expect(screen.getByText('未通过')).toBeInTheDocument()
    expect(screen.getByText('邮箱服务器拒绝了账号登录，请检查密码或授权码。')).toBeInTheDocument()
    expect(screen.queryByText('登录验证通过，尚未测试实际投递。')).not.toBeInTheDocument()
    expect(onSubmit).not.toHaveBeenCalled()
    expect(screen.queryByRole('button', { name: '手工设置' })).not.toBeInTheDocument()
  })

  it('signs in straight away without waiting for the debounced lookup', async () => {
    const client = testClient()
    client.discoverMailSettings.mockResolvedValue(ewsDiscovery())
    const { onSubmit } = renderSignInForm(client)
    fillSignIn()
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce(), WAIT)
    // The lookup ran on the click itself, password included, not only after the debounce.
    expect(client.discoverMailSettings.mock.calls[0][0]).toMatchObject({
      emailAddress: 'alice@example.com', password: 'synthetic-secret', route: { kind: 'direct' },
    })
    expect(client.verifyDraftConnection).toHaveBeenCalledOnce()
    const [connection] = onSubmit.mock.calls[0]
    expect(connection).toMatchObject({
      emailAddress: 'alice@example.com',
      ews: { url: EWS_URL, auth: 'ntlm', username: 'alice@example.com', certPrincipalName: null },
      imap: { host: 'mail.example.com', port: 443, tls: 'implicit', username: 'alice@example.com' },
      smtp: { host: 'mail.example.com', port: 443, tls: 'implicit', username: 'alice@example.com' },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-secret' } },
    })
  })

  it('falls back to the manual form with the lookup failure when nothing is found', async () => {
    const client = testClient()
    client.discoverMailSettings.mockResolvedValue({ status: 'notFound', imap: null, smtp: null, ews: null, source: null })
    const { onSubmit } = renderSignInForm(client)
    fillSignIn()
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    expect(await screen.findByLabelText('IMAP 主机')).toBeInTheDocument()
    expect(screen.getByLabelText('SMTP 主机')).toBeInTheDocument()
    expect(screen.getByText('未找到该邮箱地址公布的服务器设置。')).toBeInTheDocument()
    expect(screen.getByText('未能自动探测，请填写收信与发信服务器信息。')).toBeInTheDocument()
    // The card is gone; the manual form saves with its own submit button.
    expect(screen.queryByRole('button', { name: '手工设置' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '保存账号' })).toBeInTheDocument()
    expect(client.verifyDraftConnection).not.toHaveBeenCalled()
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('asks for the missing address and password without contacting anything', async () => {
    const { client, onSubmit } = renderSignInForm()
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    const address = screen.getByLabelText('邮箱地址')
    expect(address).toHaveAttribute('aria-invalid', 'true')
    expect(address).toHaveAccessibleDescription('请填写有效的邮箱地址。')
    expect(client.discoverMailSettings).not.toHaveBeenCalled()
    expect(client.verifyDraftConnection).not.toHaveBeenCalled()
    expect(onSubmit).not.toHaveBeenCalled()

    fireEvent.change(address, { target: { value: 'alice@example.com' } })
    await userEvent.click(screen.getByRole('button', { name: '保存账号' }))
    const password = screen.getByLabelText('收发密码')
    expect(password).toHaveAttribute('aria-invalid', 'true')
    expect(password).toHaveAccessibleDescription('请重新输入密码或授权码，空白不会覆盖已保存的凭证。')
    expect(client.discoverMailSettings).not.toHaveBeenCalled()
    expect(client.verifyDraftConnection).not.toHaveBeenCalled()
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('prefills an Exchange endpoint from the address domain and switches back to IMAP/SMTP', async () => {
    renderSignInForm()
    await userEvent.click(screen.getByRole('button', { name: '手工设置' }))
    fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: 'alice@example.com' } })
    await userEvent.click(screen.getByRole('tab', { name: 'Exchange' }))
    expect(screen.getByLabelText('Exchange 服务器 URL')).toHaveValue(EWS_URL)
    expect(screen.getByLabelText('登录名')).toHaveValue('alice@example.com')
    expect(screen.getByRole('combobox', { name: '认证方式' })).toHaveTextContent('NTLM')
    expect(screen.queryByLabelText('IMAP 主机')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('tab', { name: 'IMAP' }))
    expect(screen.queryByLabelText('Exchange 服务器 URL')).not.toBeInTheDocument()
    expect(screen.getByLabelText('IMAP 主机')).toBeInTheDocument()
    expect(screen.getByLabelText('SMTP 主机')).toBeInTheDocument()
  })

  it('offers POP3 only as a disabled choice', async () => {
    renderSignInForm()
    await userEvent.click(screen.getByRole('button', { name: '手工设置' }))
    expect(screen.getByRole('tab', { name: 'POP3' })).toHaveAttribute('aria-disabled', 'true')
  })

  it('shows the login test while editing Exchange details', async () => {
    renderSignInForm()
    await userEvent.click(screen.getByRole('button', { name: '手工设置' }))
    fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: 'alice@example.com' } })
    await userEvent.click(screen.getByRole('tab', { name: 'Exchange' }))
    expect(screen.getByRole('button', { name: '测试登录' })).toBeInTheDocument()
  })

  it('edits an existing account on the server form without a sign-in card', () => {
    render(
      <ConnectionForm account={exampleAccount()} discoveryClient={testClient()}
        intro={{ title: '说明', description: '说明' }} submitLabel="保存更改" submittingLabel="保存中…"
        onSubmit={vi.fn()} onBusyChange={vi.fn()} onDirtyChange={vi.fn()} />,
    )
    expect(screen.getByLabelText('IMAP 主机')).toBeInTheDocument()
    expect(screen.getByLabelText('SMTP 主机')).toBeInTheDocument()
    expect(screen.queryByText('登录邮箱')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '手工设置' })).not.toBeInTheDocument()
  })
})
