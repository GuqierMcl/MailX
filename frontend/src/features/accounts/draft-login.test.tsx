import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { i18n } from '@/i18n'
import { AddAccountDialog } from './add-account-dialog'
import { FEISHU_PROVIDER, GENERIC_PROVIDER, LARK_PROVIDER, MAIL_139_PROVIDER, NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER, QQ_PROVIDER, SINA_PROVIDER, type MailProvider } from './providers'
import { testClient } from './test-fixtures'

type Report = Awaited<ReturnType<MailxClient['verifyDraftConnection']>>
const success: Report = {
  verificationId: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
  imap: { status: 'verified', stage: 'complete', code: null },
  smtp: { status: 'verified', stage: 'complete', code: null },
}
const succeeded = '登录验证通过，尚未测试实际投递。'
const regionalPresets = [
  { provider: FEISHU_PROVIDER, serverDomain: 'feishu.cn' },
  { provider: LARK_PROVIDER, serverDomain: 'larksuite.com' },
]

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

function credentialLabel(provider: MailProvider) {
  return provider.setup?.credentialKind === 'appPassword' ? '应用专用密码' : provider.setup ? '授权码' : '收发密码'
}

function defaultDomain(provider: MailProvider) {
  const domains = provider.preset?.domains
  return domains && domains !== 'organization' ? domains[0] : 'example.com'
}

function fill(provider: MailProvider = GENERIC_PROVIDER, emailAddress = `Synthetic@${defaultDomain(provider)}`, credential = 'synthetic-authorization-code', manualServerDomain = 'example.com') {
  fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value: emailAddress } })
  if (!provider.preset) {
    fireEvent.change(screen.getByLabelText('IMAP 主机'), { target: { value: `imap.${manualServerDomain}` } })
    fireEvent.change(screen.getByLabelText('SMTP 主机'), { target: { value: `smtp.${manualServerDomain}` } })
  }
  fireEvent.change(screen.getByLabelText(credentialLabel(provider)), { target: { value: credential } })
  return emailAddress
}

async function chooseOption(label: string, option: string) {
  await userEvent.click(screen.getByRole('combobox', { name: label }))
  await userEvent.click(await screen.findByRole('option', { name: option }))
}

describe('testing a connection before saving an account', () => {
  it.each([
    ...[QQ_PROVIDER, NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER, MAIL_139_PROVIDER, GENERIC_PROVIDER]
      .map((provider) => ({ provider, domain: defaultDomain(provider), serverDomain: defaultDomain(provider) })),
    { provider: SINA_PROVIDER, domain: 'SINA.COM', serverDomain: 'sina.com' },
    { provider: SINA_PROVIDER, domain: 'SINA.CN', serverDomain: 'sina.cn' },
    { provider: GENERIC_PROVIDER, domain: 'FASTMAIL.COM', serverDomain: 'fastmail.com' },
    ...regionalPresets.map((preset) => ({ ...preset, domain: 'Mail.Example.COM' })),
  ])('tests $provider.id for $domain without creating an account and retains the credential for the final save', async ({ provider, domain, serverDomain }) => {
    const { client, onCreated } = setup()
    await select(provider)
    const credential = ' synthetic-authorization-code '
    const emailAddress = fill(provider, `Synthetic@${domain}`, credential, serverDomain)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    const result = await screen.findByText(succeeded)
    const report = within(result.closest('[role="status"]')!)
    expect(report.getByText(/IMAP/)).toBeInTheDocument()
    expect(report.getByText(/SMTP/)).toBeInTheDocument()
    expect(report.getAllByText('已通过')).toHaveLength(2)
    expect(client.verifyDraftConnection).toHaveBeenCalledOnce()
    const [connection, signal] = client.verifyDraftConnection.mock.calls[0]
    expect(connection).toMatchObject({
      emailAddress, providerId: provider.id,
      imap: { host: `imap.${serverDomain}`, port: 993, tls: 'implicit', username: emailAddress },
      smtp: { host: `smtp.${serverDomain}`, port: 465, tls: 'implicit', username: emailAddress },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: credential } },
      route: { kind: 'direct' },
    })
    expect(signal).toBeInstanceOf(AbortSignal)
    expect(screen.getByLabelText(credentialLabel(provider))).toHaveValue(credential)
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
    expect(client.verifyDraftConnection).toHaveBeenCalledOnce()
  })

  it('updates both Sina defaults on address change, preserves an override, and resumes with the tested connection', async () => {
    const { client, onCreated } = setup()
    await select(SINA_PROVIDER)
    fill(SINA_PROVIDER, 'Synthetic@SINA.COM')
    await userEvent.click(screen.getByText('高级设置'))
    expect(screen.getByLabelText('IMAP 主机')).toHaveValue('imap.sina.com')
    expect(screen.getByLabelText('SMTP 主机')).toHaveValue('smtp.sina.com')
    const email = screen.getByLabelText('邮箱地址')
    fireEvent.change(email, { target: { value: ' Synthetic@SINA.CN ' } })
    expect(screen.getByLabelText('IMAP 主机')).toHaveValue('imap.sina.cn')
    expect(screen.getByLabelText('SMTP 主机')).toHaveValue('smtp.sina.cn')
    fireEvent.change(screen.getByLabelText('IMAP 主机'), { target: { value: 'custom.example.test' } })
    fireEvent.change(email, { target: { value: ' Synthetic@SINA.COM ' } })
    expect(screen.getByLabelText('IMAP 主机')).toHaveValue('custom.example.test')
    expect(screen.getByLabelText('SMTP 主机')).toHaveValue('smtp.sina.com')
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    expect(await screen.findByText(succeeded)).toBeInTheDocument()
    const [tested] = client.verifyDraftConnection.mock.calls[0]
    expect(tested).toMatchObject({
      emailAddress: 'Synthetic@SINA.COM',
      imap: { host: 'custom.example.test', username: 'Synthetic@SINA.COM' }, smtp: { host: 'smtp.sina.com' },
    })
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    await userEvent.click(screen.getByRole('button', { name: '上一步' }))
    expect(screen.getByLabelText('IMAP 主机')).toHaveValue('custom.example.test')
    expect(screen.getByLabelText('SMTP 主机')).toHaveValue('smtp.sina.com')
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    await userEvent.click(screen.getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject(tested)
  })

  it.each(regionalPresets)('shows fixed new $provider.id endpoints while keeping the route configurable', async ({ provider, serverDomain }) => {
    setup()
    await select(provider)
    fill(provider, 'member@mail.example.com')
    await userEvent.click(screen.getByText('高级设置'))
    for (const [protocol, host, port] of [['IMAP', `imap.${serverDomain}`, '993'], ['SMTP', `smtp.${serverDomain}`, '465']]) {
      expect(screen.getByLabelText(`${protocol} 主机`)).toHaveValue(host)
      expect(screen.getByLabelText(`${protocol} 主机`)).toHaveAttribute('readonly')
      expect(screen.getByLabelText(`${protocol} 端口`)).toHaveValue(port)
      expect(screen.getByLabelText(`${protocol} 端口`)).toHaveAttribute('readonly')
      expect(screen.getByRole('combobox', { name: `${protocol} 连接安全` })).toBeDisabled()
    }
    expect(screen.getByRole('combobox', { name: '连接方式' })).toBeEnabled()
  })

  it.each([
    { provider: SINA_PROVIDER, address: 'me@vip.sina.com', guidance: '新浪 VIP 邮箱请使用手动配置', english: 'For Sina VIP Mail, choose manual setup' },
    { provider: FEISHU_PROVIDER, address: 'me@company.com/path', guidance: '请填写有效的中国版飞书成员邮箱地址', english: 'Enter a valid Feishu China member address' },
    { provider: LARK_PROVIDER, address: 'me@company.com/path', guidance: '请填写有效的 Lark 成员邮箱地址', english: 'Enter a valid Lark member address' },
  ])('blocks testing and creating $provider.id for $address, with localized correction', async ({ provider, address, guidance, english }) => {
    const { client, dialog } = setup()
    await select(provider)
    fill(provider, address)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    expect(screen.getByLabelText('邮箱地址')).toHaveAttribute('aria-invalid', 'true')
    expect(dialog).toHaveTextContent(guidance)
    expect(screen.getByLabelText(credentialLabel(provider))).toHaveValue('synthetic-authorization-code')
    expect(client.verifyDraftConnection).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    expect(screen.queryByLabelText('账号名称')).not.toBeInTheDocument()
    expect(client.createAccount).not.toHaveBeenCalled()
    try {
      await act(async () => { await i18n.changeLanguage('en') })
      expect(dialog).toHaveTextContent(english)
    } finally {
      await act(async () => { await i18n.changeLanguage('zh-CN') })
    }
  })

  it.each([
    { provider: SINA_PROVIDER, serverDomain: 'sina.com', code: 'authentication', advice: /确认 IMAP4\/SMTP 已开启/, help: '如何获取授权码' },
    { provider: FEISHU_PROVIDER, serverDomain: 'feishu.cn', code: 'providerPolicy', advice: /请联系飞书管理员/, help: '查看官方帮助' },
    { provider: LARK_PROVIDER, serverDomain: 'larksuite.com', code: 'providerPolicy', advice: /^请联系 Lark 管理员/, help: '查看官方帮助' },
  ] as const)('sends one test to $serverDomain and opens only $provider.id help on failure', async ({ provider, serverDomain, code, advice, help }) => {
    const client = testClient()
    client.verifyDraftConnection.mockResolvedValue({ ...success, imap: { status: 'failed', stage: 'authentication', code } })
    setup(client)
    await select(provider)
    fill(provider)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    const report = (await screen.findByText(advice)).closest<HTMLElement>('[role="status"]')!
    await userEvent.click(within(report).getByRole('button', { name: help }))
    expect(client.openProviderHelp).toHaveBeenCalledExactlyOnceWith(provider.id)
    expect(client.verifyDraftConnection).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      providerId: provider.id, imap: expect.objectContaining({ host: `imap.${serverDomain}` }), smtp: expect.objectContaining({ host: `smtp.${serverDomain}` }),
    }), expect.any(AbortSignal))
    expect(client.verifyAccountConnection).not.toHaveBeenCalled()
    expect(client.createAccount).not.toHaveBeenCalled()
  })

  it('keeps IMAP success separate from SMTP failure, offers authorization help, and allows continuing', async () => {
    const client = testClient()
    client.verifyDraftConnection.mockResolvedValue({ ...success, smtp: { status: 'failed', stage: 'authentication', code: 'authentication' } })
    setup(client)
    await select(NETEASE_163_PROVIDER)
    fill(NETEASE_163_PROVIDER)
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    const reason = await screen.findByText('邮箱服务器拒绝了账号登录，请检查密码或授权码。')
    expect(reason).toHaveClass('text-xs', 'text-destructive')
    const report = within(reason.closest('[role="status"]')!)
    expect(report.getAllByText('已通过')).toHaveLength(1)
    expect(report.getByText('未通过')).toBeInTheDocument()
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

  it.each([{ provider: QQ_PROVIDER, serverDomain: 'qq.com' }, ...regionalPresets])('cancels a running $provider.id test without fallback and retries only on request', async ({ provider, serverDomain }) => {
    const client = testClient()
    const pending = deferred<Report>()
    client.verifyDraftConnection.mockReturnValueOnce(pending.promise).mockResolvedValue(success)
    setup(client)
    await select(provider)
    fill(provider)
    const test = screen.getByRole('button', { name: '测试登录' })
    fireEvent.click(test)
    fireEvent.click(test)
    expect(client.verifyDraftConnection).toHaveBeenCalledOnce()
    const [connection, signal] = client.verifyDraftConnection.mock.calls[0]
    expect(connection).toMatchObject({ providerId: provider.id, imap: { host: `imap.${serverDomain}` }, smtp: { host: `smtp.${serverDomain}` } })
    expect(screen.getByRole('button', { name: '正在测试…' })).toBeDisabled()
    expect(screen.getByLabelText(credentialLabel(provider))).toHaveValue('synthetic-authorization-code')
    await userEvent.click(screen.getByRole('button', { name: '取消测试' }))
    expect(signal?.aborted).toBe(true)
    await act(async () => pending.resolve(success))
    expect(screen.queryByText(succeeded)).not.toBeInTheDocument()
    expect(client.verifyDraftConnection).toHaveBeenCalledOnce()
    expect(client.verifyAccountConnection).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '测试登录' }))
    expect(await screen.findByText(succeeded)).toBeInTheDocument()
    expect(client.verifyDraftConnection).toHaveBeenCalledTimes(2)
    expect(client.verifyDraftConnection.mock.calls[1][0]).toEqual(connection)
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
    const checked = client.verifyDraftConnection.mock.calls[1][0]
    if (!('imap' in checked)) throw new Error('IMAP fixture')
    expect(checked.imap.username).toBe('changed-user')
    await act(async () => pending.resolve({ ...success, smtp: { status: 'failed', stage: 'authentication', code: 'authentication' } }))
    expect(screen.getByText(succeeded)).toBeInTheDocument()
    expect(screen.queryByText('未通过')).not.toBeInTheDocument()
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
