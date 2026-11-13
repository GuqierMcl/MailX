import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { MailSettingsDiscovery, MailSettingsDiscoveryStatus } from '@/generated/accounts'
import { AddAccountDialog } from './add-account-dialog'
import { ConnectionForm } from './connection-form'
import { exampleAccount, testClient } from './test-fixtures'

const EWS_URL = 'https://mail.example.com/EWS/Exchange.asmx'
const SOURCE = 'autodiscover.example.com'
const WAIT = { timeout: 3000 }

function discovery(overrides: Partial<MailSettingsDiscovery> = {}): MailSettingsDiscovery {
  return { status: 'notFound', imap: null, smtp: null, ews: null, source: null, ...overrides }
}

function imapSmtpFound(): MailSettingsDiscovery {
  return discovery({
    status: 'found',
    imap: { host: 'imap.example.com', port: 993, tls: 'implicit', username: null },
    smtp: { host: 'smtp.example.com', port: 587, tls: 'startTls', username: 'alice' },
    source: SOURCE,
  })
}

function ewsFound(): MailSettingsDiscovery {
  return discovery({
    status: 'found',
    ews: { url: EWS_URL, auth: 'ntlm', username: 'EXAMPLE\\alice', certPrincipalName: 'msstd:mail.example.com' },
    source: SOURCE,
  })
}

function renderDialog(client = testClient()) {
  const onCreated = vi.fn()
  render(<AddAccountDialog client={client} open accounts={[]} onClose={vi.fn()} onCreated={onCreated} />)
  return { client, onCreated, dialog: screen.getByRole('dialog', { name: '添加邮箱账号' }) }
}

async function chooseImap(dialog: HTMLElement) {
  await userEvent.click(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ }))
  await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
}

function enterAddress(dialog: HTMLElement, address = 'alice@example.com') {
  fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: address } })
}

describe('automatic server discovery for IMAP/SMTP', () => {
  it('starts a lookup after an address is entered, without asking for servers first', async () => {
    const { client, dialog } = renderDialog()
    await chooseImap(dialog)
    expect(within(dialog).queryByLabelText('IMAP 主机')).not.toBeInTheDocument()
    expect(within(dialog).getByText('输入邮箱地址后，MailX 会自动探测服务器设置。')).toBeInTheDocument()
    enterAddress(dialog)
    await waitFor(() => expect(client.discoverMailSettings).toHaveBeenCalledWith({
      emailAddress: 'alice@example.com', password: null, route: { kind: 'direct' },
    }), WAIT)
    expect(within(dialog).getByLabelText('IMAP 主机')).toBeInTheDocument()
  })

  it('shows a detecting status while the lookup is in flight', async () => {
    const client = testClient()
    let release!: (value: MailSettingsDiscovery) => void
    client.discoverMailSettings.mockReturnValue(new Promise((resolve) => { release = resolve }))
    const { dialog } = renderDialog(client)
    await chooseImap(dialog)
    enterAddress(dialog)
    expect(await within(dialog).findByText('正在探测服务器设置…', {}, WAIT)).toBeInTheDocument()
    await act(async () => { release(discovery({ status: 'notFound' })) })
    expect(await within(dialog).findByText('未找到该邮箱地址公布的服务器设置。')).toBeInTheDocument()
  })

  it('fills IMAP/SMTP fields and shows the source when settings are found', async () => {
    const client = testClient()
    client.discoverMailSettings.mockResolvedValue(imapSmtpFound())
    const { dialog } = renderDialog(client)
    await chooseImap(dialog)
    enterAddress(dialog)
    await waitFor(() => expect(within(dialog).getByLabelText('IMAP 主机')).toHaveValue('imap.example.com'), WAIT)
    expect(within(dialog).getByLabelText('IMAP 端口')).toHaveValue('993')
    expect(within(dialog).getByLabelText('IMAP 登录用户名')).toHaveValue('')
    expect(within(dialog).getByLabelText('SMTP 主机')).toHaveValue('smtp.example.com')
    expect(within(dialog).getByLabelText('SMTP 端口')).toHaveValue('587')
    expect(within(dialog).getByLabelText('SMTP 登录用户名')).toHaveValue('alice')
    expect(within(dialog).getByText('已自动探测到服务器设置，仍可手动修改。')).toBeInTheDocument()
    expect(within(dialog).getByText(`探测来源：${SOURCE}`)).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '重新探测' })).toBeInTheDocument()
  })

  it('hides server fields, shows Exchange/NTLM details and submits the detected ews config', async () => {
    const client = testClient()
    client.discoverMailSettings.mockResolvedValue(ewsFound())
    const { dialog, onCreated } = renderDialog(client)
    await chooseImap(dialog)
    enterAddress(dialog, 'alice@contoso.com')
    await waitFor(() => expect(dialog).toHaveTextContent(EWS_URL), WAIT)
    expect(within(dialog).queryByLabelText('IMAP 主机')).not.toBeInTheDocument()
    expect(within(dialog).queryByLabelText('SMTP 主机')).not.toBeInTheDocument()
    expect(within(dialog).queryByRole('radio', { name: '收发分别设置密码' })).not.toBeInTheDocument()
    expect(within(dialog).queryByRole('button', { name: '测试登录' })).not.toBeInTheDocument()
    expect(within(dialog).getByText('通过 HTTPS 使用 Exchange Web Services 收发邮件。')).toBeInTheDocument()
    expect(within(dialog).getByText('认证方式: NTLM')).toBeInTheDocument()
    expect(within(dialog).getByText('登录名: EXAMPLE\\alice')).toBeInTheDocument()
    expect(within(dialog).getByText('证书主体: msstd:mail.example.com')).toBeInTheDocument()
    expect(within(dialog).getByText(`探测来源：${SOURCE}`)).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '改为手动填写' })).toBeInTheDocument()

    fireEvent.change(within(dialog).getByLabelText('收发密码'), { target: { value: 'synthetic-secret' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByLabelText('账号名称')).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce(), WAIT)
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject({
      ews: { url: EWS_URL, auth: 'ntlm', username: 'EXAMPLE\\alice', certPrincipalName: 'msstd:mail.example.com' },
      imap: { host: 'mail.example.com', port: 443, tls: 'implicit', username: 'EXAMPLE\\alice' },
      smtp: { host: 'mail.example.com', port: 443, tls: 'implicit', username: 'EXAMPLE\\alice' },
    })
  })

  it.each<[MailSettingsDiscoveryStatus, string]>([
    ['rejected', '服务器拒绝了探测请求。'],
    ['notFound', '未找到该邮箱地址公布的服务器设置。'],
    ['network', '无法连接服务器以探测设置。'],
    ['proxy', '代理阻止了探测请求。'],
    ['tls', '探测连接无法建立安全连接。'],
    ['timeout', '探测请求超时。'],
    ['unsupported', '该邮箱地址不支持自动探测。'],
  ])('reports %s and falls back to editable IMAP/SMTP fields', async (status, text) => {
    const client = testClient()
    client.discoverMailSettings.mockResolvedValue(discovery({ status }))
    const { dialog } = renderDialog(client)
    await chooseImap(dialog)
    enterAddress(dialog)
    expect(await within(dialog).findByText(text, {}, WAIT)).toBeInTheDocument()
    expect(within(dialog).getByText('未能自动探测，请填写收信与发信服务器信息。')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('IMAP 主机')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('SMTP 主机')).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '重新探测' })).toBeInTheDocument()
  })

  it('retries with the entered password after needsCredentials', async () => {
    const client = testClient()
    client.discoverMailSettings
      .mockResolvedValueOnce(discovery({ status: 'needsCredentials' }))
      .mockResolvedValueOnce(imapSmtpFound())
    const { dialog } = renderDialog(client)
    await chooseImap(dialog)
    enterAddress(dialog)
    expect(await within(dialog).findByText('服务器需要验证身份后才能提供设置。', {}, WAIT)).toBeInTheDocument()
    expect(within(dialog).getByText('探测需要密码，请输入密码后重试。')).toBeInTheDocument()
    expect(client.discoverMailSettings.mock.calls[0][0].password).toBeNull()
    fireEvent.change(within(dialog).getByLabelText('收发密码'), { target: { value: 'synthetic-secret' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '输入密码后重试' }))
    await waitFor(() => expect(client.discoverMailSettings).toHaveBeenCalledTimes(2), WAIT)
    expect(client.discoverMailSettings.mock.calls[1][0].password).toBe('synthetic-secret')
    expect(await within(dialog).findByText('已自动探测到服务器设置，仍可手动修改。')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('IMAP 主机')).toHaveValue('imap.example.com')
  })

  it('discards a stale result and looks up again after the address or route changes', async () => {
    const client = testClient()
    client.discoverMailSettings
      .mockResolvedValueOnce(imapSmtpFound())
      .mockResolvedValue(discovery({ status: 'notFound' }))
    const { dialog } = renderDialog(client)
    await chooseImap(dialog)
    enterAddress(dialog)
    expect(await within(dialog).findByText('已自动探测到服务器设置，仍可手动修改。', {}, WAIT)).toBeInTheDocument()
    fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: 'bob@example.com' } })
    await waitFor(() => expect(client.discoverMailSettings).toHaveBeenCalledTimes(2), WAIT)
    expect(client.discoverMailSettings.mock.calls[1][0].emailAddress).toBe('bob@example.com')
    expect(within(dialog).getByText('未找到该邮箱地址公布的服务器设置。')).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('combobox', { name: '连接方式' }))
    await userEvent.click(await screen.findByRole('option', { name: '使用应用默认' }))
    await waitFor(() => expect(client.discoverMailSettings).toHaveBeenCalledTimes(3), WAIT)
    expect(client.discoverMailSettings.mock.calls[2][0].route).toEqual({ kind: 'inherit' })
  })

  it('does not auto-discover when editing an existing account but offers a manual retry', async () => {
    const client = testClient()
    render(<ConnectionForm account={exampleAccount()} discoveryClient={client}
      intro={{ title: '说明', description: '说明' }} submitLabel="保存更改" submittingLabel="保存中…"
      onSubmit={vi.fn()} onBusyChange={vi.fn()} onDirtyChange={vi.fn()} />)
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 400)) })
    expect(client.discoverMailSettings).not.toHaveBeenCalled()
    expect(screen.getByLabelText('IMAP 主机')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重新探测' }))
    await waitFor(() => expect(client.discoverMailSettings).toHaveBeenCalledOnce(), WAIT)
    expect(client.discoverMailSettings.mock.calls[0][0].emailAddress).toBe('work@example.com')
  })

  it('switches back to manual IMAP/SMTP fields from the EWS summary', async () => {
    const client = testClient()
    client.discoverMailSettings.mockResolvedValue(discovery({
      status: 'found',
      ews: { url: EWS_URL, auth: 'basic', username: null, certPrincipalName: null },
      source: SOURCE,
    }))
    const { dialog, onCreated } = renderDialog(client)
    await chooseImap(dialog)
    enterAddress(dialog, 'alice@contoso.com')
    await waitFor(() => expect(within(dialog).getByText('认证方式: Basic')).toBeInTheDocument(), WAIT)
    await userEvent.click(within(dialog).getByRole('button', { name: '改为手动填写' }))
    expect(within(dialog).getByLabelText('IMAP 主机')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('SMTP 主机')).toBeInTheDocument()
    expect(within(dialog).queryByText('通过 HTTPS 使用 Exchange Web Services 收发邮件。')).not.toBeInTheDocument()
    fireEvent.change(within(dialog).getByLabelText('收发密码'), { target: { value: 'synthetic-secret' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByLabelText('账号名称')).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce(), WAIT)
    expect(client.createAccount.mock.calls[0][0].config.ews).toBeNull()
  })
})
