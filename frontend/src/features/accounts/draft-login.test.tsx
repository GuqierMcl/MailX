import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { AddAccountDialog } from './add-account-dialog'
import { GENERIC_PROVIDER, NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER, QQ_PROVIDER, type MailProvider } from './providers'
import { testClient } from './test-fixtures'

type Report = Awaited<ReturnType<MailxClient['verifyDraftConnection']>>
const success: Report = {
  verificationId: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
  imap: { status: 'verified', stage: 'complete', code: null },
  smtp: { status: 'verified', stage: 'complete', code: null },
}
const succeeded = '登录验证通过，尚未测试实际投递。'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

function setup(client = testClient()) {
  const props = { client, accounts: [], onClose: vi.fn(), onCreated: vi.fn() }
  const view = render(<AddAccountDialog {...props} open />)
  return { ...view, ...props, dialog: screen.getByRole('dialog', { name: '添加邮箱账号' }) }
}

async function select(provider: MailProvider) {
  await userEvent.click(screen.getByRole('radio', { name: provider.name }))
  await userEvent.click(screen.getByRole('button', { name: '下一步' }))
}

function fill(provider: MailProvider = GENERIC_PROVIDER) {
  const domain = provider.preset?.domains[0] ?? 'example.com'
  const emailAddress = `Synthetic@${domain}`
  fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: emailAddress } })
  if (!provider.preset) {
    fireEvent.change(screen.getByLabelText('IMAP 主机'), { target: { value: 'imap.example.com' } })
    fireEvent.change(screen.getByLabelText('SMTP 主机'), { target: { value: 'smtp.example.com' } })
  }
  fireEvent.change(screen.getByLabelText(provider.setup ? '授权码' : '收发密码'), { target: { value: 'synthetic-authorization-code' } })
  return emailAddress
}

async function chooseOption(label: string, option: string) {
  await userEvent.click(screen.getByRole('combobox', { name: label }))
  await userEvent.click(await screen.findByRole('option', { name: option }))
}

describe('testing a connection before saving an account', () => {
  it.each([QQ_PROVIDER, NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER, GENERIC_PROVIDER])('tests $name without creating an account and retains its credential for the final save', async (provider) => {
    const { client, onCreated } = setup()
    await select(provider)
    const emailAddress = fill(provider)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    const result = await screen.findByText(succeeded)
    const report = within(result.closest('[role="status"]')!)
    expect(report.getByText(/IMAP/)).toBeInTheDocument()
    expect(report.getByText(/SMTP/)).toBeInTheDocument()
    expect(report.getAllByText('登录成功')).toHaveLength(2)
    expect(client.verifyDraftConnection).toHaveBeenCalledOnce()
    const [connection, signal] = client.verifyDraftConnection.mock.calls[0]
    expect(connection).toMatchObject({
      emailAddress, providerId: provider.id,
      imap: { host: `imap.${provider.preset?.domains[0] ?? 'example.com'}`, port: 993, tls: 'implicit', username: emailAddress },
      smtp: { host: `smtp.${provider.preset?.domains[0] ?? 'example.com'}`, port: 465, tls: 'implicit', username: emailAddress },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-authorization-code' } },
      route: { kind: 'direct' },
    })
    expect(signal).toBeInstanceOf(AbortSignal)
    expect(screen.getByLabelText(provider.setup ? '授权码' : '收发密码')).toHaveValue('synthetic-authorization-code')
    expect(client.createAccount).not.toHaveBeenCalled()
    expect(client.updateConnection).not.toHaveBeenCalled()
    expect(client.verifyAccountConnection).not.toHaveBeenCalled()
    expect(client.refreshSync).not.toHaveBeenCalled()
    expect(client.queueSend).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    await screen.findByLabelText('账号名称')
    await userEvent.click(screen.getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount).toHaveBeenCalledOnce()
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject(connection)
  })

  it('keeps IMAP success separate from SMTP failure, offers authorization help, and allows continuing', async () => {
    const client = testClient()
    client.verifyDraftConnection.mockResolvedValue({ ...success, smtp: { status: 'failed', stage: 'authentication', code: 'authentication' } })
    setup(client)
    await select(NETEASE_163_PROVIDER)
    fill(NETEASE_163_PROVIDER)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    const result = await screen.findByText('登录验证未通过')
    const report = within(result.closest('[role="status"]')!)
    expect(report.getAllByText('登录成功')).toHaveLength(1)
    expect(report.getByText('邮箱服务器拒绝了账号登录，请检查密码或授权码。')).toBeInTheDocument()
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
    const help = screen.getAllByRole('button', { name: '如何获取授权码' }).at(-1)!
    await userEvent.click(help)
    expect(client.openProviderHelp).toHaveBeenCalledWith('netease163')
    expect(screen.getByLabelText('授权码')).toHaveValue('synthetic-authorization-code')
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    expect(await screen.findByLabelText('账号名称')).toBeInTheDocument()
    expect(client.createAccount).not.toHaveBeenCalled()
  })

  it('focuses invalid local fields without contacting either protocol or erasing the current credential', async () => {
    const { client } = setup()
    await select(NETEASE_163_PROVIDER)
    fill(NETEASE_163_PROVIDER)
    await userEvent.click(screen.getByText('高级设置'))
    fireEvent.change(screen.getByLabelText('IMAP 端口'), { target: { value: '0' } })
    await userEvent.click(screen.getByText('高级设置'))
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    const port = screen.getByLabelText('IMAP 端口')
    await waitFor(() => {
      expect(port).toHaveAttribute('aria-invalid', 'true')
      expect(port).toBeVisible()
      expect(port).toHaveFocus()
    })
    expect(screen.getByLabelText('授权码')).toHaveValue('synthetic-authorization-code')
    expect(client.verifyDraftConnection).not.toHaveBeenCalled()
    expect(client.createAccount).not.toHaveBeenCalled()
    expect(client.refreshSync).not.toHaveBeenCalled()
  })

  it('cancels a running test, ignores its late success and permits another test with the same input', async () => {
    const client = testClient()
    const pending = deferred<Report>()
    client.verifyDraftConnection.mockReturnValueOnce(pending.promise).mockResolvedValue(success)
    setup(client)
    await select(QQ_PROVIDER)
    fill(QQ_PROVIDER)
    const test = screen.getByRole('button', { name: '测试登录' })
    fireEvent.click(test)
    fireEvent.click(test)
    expect(client.verifyDraftConnection).toHaveBeenCalledOnce()
    const signal = client.verifyDraftConnection.mock.calls[0][1]!
    expect(screen.getByRole('button', { name: '正在测试…' })).toBeDisabled()
    expect(screen.getByLabelText('授权码')).toHaveValue('synthetic-authorization-code')
    await userEvent.click(screen.getByRole('button', { name: '取消测试' }))
    expect(signal.aborted).toBe(true)
    await act(async () => pending.resolve(success))
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    expect(await screen.findByText(succeeded)).toBeInTheDocument()
    expect(client.verifyDraftConnection).toHaveBeenCalledTimes(2)
    expect(client.verifyDraftConnection.mock.calls[1][1]!.aborted).toBe(false)
  })

  it('invalidates a pending test when a connection field changes and clears a completed report after a later edit', async () => {
    const client = testClient()
    const pending = deferred<Report>()
    client.verifyDraftConnection.mockReturnValueOnce(pending.promise).mockResolvedValue(success)
    setup(client)
    await select(GENERIC_PROVIDER)
    fill()
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    const signal = client.verifyDraftConnection.mock.calls[0][1]!
    fireEvent.change(screen.getByLabelText('IMAP 登录用户名'), { target: { value: 'changed-user' } })
    expect(signal.aborted).toBe(true)
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    expect(await screen.findByText(succeeded)).toBeInTheDocument()
    expect(client.verifyDraftConnection.mock.calls[1][0].imap.username).toBe('changed-user')
    await act(async () => pending.resolve({ ...success, smtp: { status: 'failed', stage: 'authentication', code: 'authentication' } }))
    expect(screen.getByText(succeeded)).toBeInTheDocument()
    expect(screen.queryByText('登录验证未通过')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('收发密码'), { target: { value: 'changed-secret' } })
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
  })

  it('tests the selected proxy, encryption and separate login identities without dropping their credentials', async () => {
    const { client } = setup()
    await select(GENERIC_PROVIDER)
    fill()
    await chooseOption('SMTP 连接安全', 'STARTTLS')
    fireEvent.change(screen.getByLabelText('SMTP 端口'), { target: { value: '587' } })
    fireEvent.change(screen.getByLabelText('IMAP 登录用户名'), { target: { value: 'receive-user' } })
    fireEvent.change(screen.getByLabelText('SMTP 登录用户名'), { target: { value: 'send-user' } })
    await userEvent.click(screen.getByRole('radio', { name: '收发分别设置密码' }))
    fireEvent.change(screen.getByLabelText('收信密码'), { target: { value: 'synthetic-imap-secret' } })
    fireEvent.change(screen.getByLabelText('发信密码'), { target: { value: 'synthetic-smtp-secret' } })
    await chooseOption('连接方式', '自定义代理')
    fireEvent.change(screen.getByLabelText('代理主机'), { target: { value: 'proxy.example.com' } })
    fireEvent.change(screen.getByLabelText('代理端口'), { target: { value: '1088' } })
    await userEvent.click(screen.getByRole('switch', { name: '代理需要认证' }))
    fireEvent.change(screen.getByLabelText('代理用户名'), { target: { value: 'proxy-user' } })
    fireEvent.change(screen.getByLabelText('代理密码'), { target: { value: 'synthetic-proxy-secret' } })
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    expect(await screen.findByText(succeeded)).toBeInTheDocument()
    expect(client.verifyDraftConnection.mock.calls[0][0]).toMatchObject({
      imap: { username: 'receive-user' }, smtp: { port: 587, tls: 'startTls', username: 'send-user' },
      authentication: { kind: 'password', mode: 'separate', imapPassword: { action: 'replace', value: 'synthetic-imap-secret' }, smtpPassword: { action: 'replace', value: 'synthetic-smtp-secret' } },
      route: { kind: 'custom', proxyKind: 'socks5', host: 'proxy.example.com', port: 1088, dns: 'proxy', imageProxyDns: false,
        authentication: { username: 'proxy-user', password: { action: 'replace', value: 'synthetic-proxy-secret' } } },
    })
    expect(screen.getByLabelText('收信密码')).toHaveValue('synthetic-imap-secret')
    expect(screen.getByLabelText('发信密码')).toHaveValue('synthetic-smtp-secret')
    expect(screen.getByLabelText('代理密码')).toHaveValue('synthetic-proxy-secret')
  })

  it('can continue during a test and ignores the old report on returning from the profile', async () => {
    const client = testClient()
    const pending = deferred<Report>()
    client.verifyDraftConnection.mockReturnValue(pending.promise)
    setup(client)
    await select(NETEASE_126_PROVIDER)
    fill(NETEASE_126_PROVIDER)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    expect(await screen.findByLabelText('账号名称')).toBeInTheDocument()
    expect(client.verifyDraftConnection.mock.calls[0][1]!.aborted).toBe(true)
    await act(async () => pending.resolve(success))
    await userEvent.click(screen.getByRole('button', { name: '上一步' }))
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
    expect(screen.getByLabelText('授权码')).toHaveValue('synthetic-authorization-code')
    expect(client.createAccount).not.toHaveBeenCalled()
  })

  it('cancels when returning to providers and does not show a late report for another provider', async () => {
    const client = testClient()
    const pending = deferred<Report>()
    client.verifyDraftConnection.mockReturnValue(pending.promise)
    setup(client)
    await select(NETEASE_163_PROVIDER)
    fill(NETEASE_163_PROVIDER)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    await userEvent.click(screen.getByRole('button', { name: '上一步' }))
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '放弃修改' }))
    expect(client.verifyDraftConnection.mock.calls[0][1]!.aborted).toBe(true)
    await select(NETEASE_YEAH_PROVIDER)
    await act(async () => pending.resolve(success))
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
    expect(screen.getByLabelText('邮箱地址')).toHaveValue('')
    expect(screen.getByLabelText('授权码')).toHaveValue('')
  })

  it('cancels on a close attempt even if editing resumes, and clears input on a later fresh opening', async () => {
    const client = testClient()
    const pending = deferred<Report>()
    client.verifyDraftConnection.mockReturnValue(pending.promise)
    const { rerender, onClose, onCreated } = setup(client)
    await select(QQ_PROVIDER)
    fill(QQ_PROVIDER)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' })
    const confirm = await screen.findByRole('alertdialog')
    await waitFor(() => expect(client.verifyDraftConnection.mock.calls[0][1]!.aborted).toBe(true))
    await userEvent.click(within(confirm).getByRole('button', { name: '继续编辑' }))
    await act(async () => pending.resolve(success))
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
    expect(screen.getByLabelText('授权码')).toHaveValue('synthetic-authorization-code')
    expect(onClose).not.toHaveBeenCalled()
    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' })
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '放弃修改' }))
    expect(onClose).toHaveBeenCalledOnce()
    rerender(<AddAccountDialog client={client} accounts={[]} onClose={onClose} onCreated={onCreated} open={false} />)
    rerender(<AddAccountDialog client={client} accounts={[]} onClose={onClose} onCreated={onCreated} open />)
    await select(QQ_PROVIDER)
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
    expect(screen.getByLabelText('授权码')).toHaveValue('')
  })

  it('shows a controlled failure message while retaining inputs after a rejected test request', async () => {
    const client = testClient()
    client.verifyDraftConnection.mockRejectedValue(new MailxClientError({ code: 'internal', field: null, message: 'PRIVATE_SYNTHETIC_SERVER_RESPONSE' }))
    const { dialog } = setup(client)
    await select(QQ_PROVIDER)
    fill(QQ_PROVIDER)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    expect(await screen.findByText('操作未完成，请重试。')).toBeInTheDocument()
    expect(dialog).not.toHaveTextContent('PRIVATE_SYNTHETIC_SERVER_RESPONSE')
    expect(screen.getByLabelText('授权码')).toHaveValue('synthetic-authorization-code')
    expect(screen.getByRole('button', { name: '下一步' })).toBeEnabled()
    expect(client.createAccount).not.toHaveBeenCalled()
  })
})
