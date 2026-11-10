import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { MailxClientError } from '@/lib/mailx-client'
import { i18n } from '@/i18n'
import { exampleAccount, testClient } from './test-fixtures'
import { AddAccountDialog } from './add-account-dialog'
import { GMAIL_PROVIDER, ICLOUD_PROVIDER, MAIL_139_PROVIDER, MICROSOFT_PERSONAL_PROVIDER, MICROSOFT_WORK_PROVIDER, NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER, type MailProvider } from './providers'

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

async function chooseQq(dialog: HTMLElement, query = 'qq') {
  fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: query } })
  await userEvent.click(within(dialog).getByRole('radio', { name: 'QQ 邮箱' }))
  await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
}

function fillQqConnection(dialog: HTMLElement, emailAddress = 'synthetic@qq.com') {
  fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: emailAddress } })
  fireEvent.change(within(dialog).getByLabelText('授权码'), { target: { value: 'syntheticcodeabc' } })
}

async function chooseProvider(dialog: HTMLElement, provider: MailProvider) {
  fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: provider.preset!.domains[0] } })
  const option = within(dialog).getByRole('radio', { name: provider.name })
  const logo = option.closest('label')!.querySelector('img')
  if (provider.logo) expect(logo).toHaveAttribute('src', provider.logo)
  else expect(logo).toBeNull()
  await userEvent.click(option)
  await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
}

async function toProfile(dialog: HTMLElement, method: 'manual' | 'qq' = 'manual') {
  if (method === 'manual') {
    await chooseImap(dialog)
    fillConnection(dialog)
  } else {
    await chooseQq(dialog)
    fillQqConnection(dialog)
  }
  await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
  return within(dialog).findByText('最后，给这个邮箱起个名字')
}

async function backToProviders(dialog: HTMLElement) {
  await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
  await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
  await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '放弃修改' }))
  fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: '' } })
}

describe('add account wizard', () => {
  it('starts with the searchable provider list and shows no connection fields yet', () => {
    const { dialog } = renderDialog()
    expect(within(dialog).getByText('第 1 步，共 3 步')).toBeInTheDocument()
    expect(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ })).toBeInTheDocument()
    expect(within(dialog).getByRole('searchbox', { name: '搜索邮箱服务商' })).toBeInTheDocument()
    expect(within(dialog).queryByLabelText('邮箱地址')).not.toBeInTheDocument()
  })

  it('selects a provider on click and moves on only with the next button', async () => {
    const { dialog } = renderDialog()
    const next = within(dialog).getByRole('button', { name: '下一步' })
    expect(next).toBeDisabled()
    const option = within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ })
    expect(option).toHaveAccessibleDescription('手动配置其他邮箱')
    await userEvent.click(option)
    expect(option).toBeChecked()
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

  it.each(['123456789@qq.com', 'synthetic@qq.com', 'synthetic@vip.qq.com', 'Synthetic@VIP.QQ.COM', 'synthetic@foxmail.com'])('saves %s with the QQ preset from only an address and authorization code', async (emailAddress) => {
    const { client, dialog, onCreated } = renderDialog()
    await chooseQq(dialog, emailAddress.split('@')[1])
    expect(within(dialog).getByText('高级设置')).toBeInTheDocument()
    expect(within(dialog).queryByLabelText('收发密码')).not.toBeInTheDocument()
    fillQqConnection(dialog, emailAddress)
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByLabelText('账号名称')).toHaveValue(emailAddress)
    expect(within(dialog).getByRole('radio', { name: '不标识' })).toHaveAttribute('aria-checked', 'true')
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount).toHaveBeenCalledOnce()
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject({
      emailAddress, providerId: 'qq', color: null, useProviderIcon: true,
      imap: { host: 'imap.qq.com', port: 993, tls: 'implicit', username: emailAddress },
      smtp: { host: 'smtp.qq.com', port: 465, tls: 'implicit', username: emailAddress },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'syntheticcodeabc' } },
      route: { kind: 'direct' },
    })
  })

  it.each([
    'Synthetic@ICLOUD.COM', 'synthetic@me.com', 'synthetic@mac.com',
  ])('tests and saves %s with the iCloud endpoints and an unchanged app password', async (emailAddress) => {
    const { client, dialog, onCreated } = renderDialog()
    await chooseProvider(dialog, ICLOUD_PROVIDER)
    expect(within(dialog).getAllByText('应用专用密码')).toHaveLength(2)
    expect(within(dialog).getByLabelText('应用专用密码')).toHaveAttribute('placeholder', '输入应用专用密码')
    expect(within(dialog).queryByLabelText('授权码')).not.toBeInTheDocument()
    expect(within(dialog).getByText(ICLOUD_PROVIDER.setup!.description)).toHaveTextContent('双重认证')
    expect(within(dialog).getByText(ICLOUD_PROVIDER.setup!.credentialHint)).toHaveTextContent('撤销全部应用专用密码')
    await userEvent.click(within(dialog).getByRole('button', { name: '如何生成应用专用密码' }))
    expect(client.openProviderHelp).toHaveBeenCalledWith('icloud')
    fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: emailAddress } })
    fireEvent.change(within(dialog).getByLabelText('应用专用密码'), { target: { value: 'abcd-efgh-ijkl-mnop' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '测试登录' }))
    expect(await within(dialog).findByText('登录验证通过，尚未测试实际投递。')).toBeInTheDocument()
    const expected = {
      emailAddress, providerId: 'icloud',
      imap: { host: 'imap.mail.me.com', port: 993, tls: 'implicit', username: emailAddress },
      smtp: { host: 'smtp.mail.me.com', port: 587, tls: 'startTls', username: emailAddress },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'abcd-efgh-ijkl-mnop' } },
      route: { kind: 'direct' },
    }
    expect(client.verifyDraftConnection.mock.calls[0][0]).toEqual(expected)
    expect(client.createAccount).not.toHaveBeenCalled()
    expect(client.queueSend).not.toHaveBeenCalled()
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByLabelText('账号名称')).toHaveValue(emailAddress)
    expect(within(dialog).getByRole('switch', { name: '使用服务商图标' })).toBeChecked()
    expect(within(dialog).getByLabelText('账号名称').parentElement!.querySelector('img')).toHaveAttribute('src', ICLOUD_PROVIDER.logo)
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject({ ...expected, useProviderIcon: true })
  })

  it.each(['Synthetic@GMAIL.COM', 'synthetic@googlemail.com'])('tests and saves %s with the Gmail endpoints and an app password', async (emailAddress) => {
    const { client, dialog, onCreated } = renderDialog()
    await chooseProvider(dialog, GMAIL_PROVIDER)
    expect(within(dialog).getByLabelText('应用专用密码')).toHaveAttribute('placeholder', '输入应用专用密码')
    expect(within(dialog).getByText(GMAIL_PROVIDER.setup!.description)).toHaveTextContent('两步验证')
    expect(within(dialog).getByText(GMAIL_PROVIDER.setup!.credentialHint)).toHaveTextContent('去掉中间的空格')
    await userEvent.click(within(dialog).getByRole('button', { name: '如何生成应用专用密码' }))
    expect(client.openProviderHelp).toHaveBeenCalledWith('gmail')
    fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: emailAddress } })
    fireEvent.change(within(dialog).getByLabelText('应用专用密码'), { target: { value: 'abcdefghijklmnop' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '测试登录' }))
    expect(await within(dialog).findByText('登录验证通过，尚未测试实际投递。')).toBeInTheDocument()
    const expected = {
      emailAddress, providerId: 'gmail',
      imap: { host: 'imap.gmail.com', port: 993, tls: 'implicit', username: emailAddress },
      smtp: { host: 'smtp.gmail.com', port: 465, tls: 'implicit', username: emailAddress },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'abcdefghijklmnop' } },
      route: { kind: 'direct' },
    }
    expect(client.verifyDraftConnection.mock.calls[0][0]).toEqual(expected)
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByLabelText('账号名称')).toHaveValue(emailAddress)
    expect(within(dialog).getByLabelText('账号名称').parentElement!.querySelector('img')).toHaveAttribute('src', GMAIL_PROVIDER.logo)
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject({ ...expected, useProviderIcon: true })
  })

  it('keeps iCloud IMAP and SMTP username overrides through the profile step', async () => {
    const { client, dialog, onCreated } = renderDialog()
    await chooseProvider(dialog, ICLOUD_PROVIDER)
    fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: 'synthetic@me.com' } })
    fireEvent.change(within(dialog).getByLabelText('应用专用密码'), { target: { value: 'abcd-efgh-ijkl-mnop' } })
    await userEvent.click(within(dialog).getByText('高级设置'))
    fireEvent.change(within(dialog).getByLabelText('IMAP 登录用户名'), { target: { value: 'synthetic' } })
    fireEvent.change(within(dialog).getByLabelText('SMTP 登录用户名'), { target: { value: 'synthetic@icloud.com' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    expect(within(dialog).getByLabelText('IMAP 登录用户名')).toHaveValue('synthetic')
    expect(within(dialog).getByLabelText('SMTP 登录用户名')).toHaveValue('synthetic@icloud.com')
    expect(within(dialog).getByLabelText('应用专用密码')).toHaveValue('abcd-efgh-ijkl-mnop')
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject({
      emailAddress: 'synthetic@me.com', imap: { username: 'synthetic' }, smtp: { username: 'synthetic@icloud.com' },
    })
  })

  it('keeps a custom domain in manual setup and clears its rejected iCloud app password', async () => {
    const { client, dialog } = renderDialog()
    await chooseProvider(dialog, ICLOUD_PROVIDER)
    fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: 'synthetic@example.com' } })
    fireEvent.change(within(dialog).getByLabelText('应用专用密码'), { target: { value: 'abcd-efgh-ijkl-mnop' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveAttribute('aria-invalid', 'true')
    expect(within(dialog).getByLabelText('应用专用密码')).toHaveValue('')
    expect(dialog).toHaveTextContent('iCloud+ 自定义域名请使用手动配置')
    expect(within(dialog).queryByLabelText('账号名称')).not.toBeInTheDocument()
    expect(client.createAccount).not.toHaveBeenCalled()
  })

  it('updates iCloud app-password labels, placeholders and help when switching languages', async () => {
    const { dialog } = renderDialog()
    await chooseProvider(dialog, ICLOUD_PROVIDER)
    try {
      await act(async () => { await i18n.changeLanguage('en') })
      expect(within(dialog).getByText('Connect iCloud Mail')).toBeInTheDocument()
      expect(within(dialog).getAllByText('App-specific password')).toHaveLength(2)
      expect(within(dialog).getByLabelText('App-specific password')).toHaveAttribute('placeholder', 'Enter your app-specific password')
      expect(within(dialog).getByRole('button', { name: 'How to generate an app-specific password' })).toBeInTheDocument()
      expect(dialog).toHaveTextContent('two-factor authentication')
      expect(within(dialog).queryByLabelText('Authorization code')).not.toBeInTheDocument()
    } finally {
      await act(async () => { await i18n.changeLanguage('zh-CN') })
    }
    expect(within(dialog).getByLabelText('应用专用密码')).toHaveAttribute('placeholder', '输入应用专用密码')
  })

  it.each([
    { provider: NETEASE_163_PROVIDER, emailAddress: 'Synthetic@163.COM', domain: '163.com' },
    { provider: NETEASE_126_PROVIDER, emailAddress: 'Synthetic@126.COM', domain: '126.com' },
    { provider: NETEASE_YEAH_PROVIDER, emailAddress: 'Synthetic@YEAH.NET', domain: 'yeah.net' },
    { provider: MAIL_139_PROVIDER, emailAddress: 'Synthetic@139.COM', domain: '139.com' },
  ])('adds $provider.id through the authorization-code flow and keeps the original login identity', async ({ provider, emailAddress, domain }) => {
    const { client, dialog, onCreated } = renderDialog()
    await chooseProvider(dialog, provider)
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveAttribute('placeholder', `you@${domain}`)
    expect(within(dialog).getByText('高级设置')).toBeInTheDocument()
    expect(within(dialog).queryByLabelText('收发密码')).not.toBeInTheDocument()
    if (provider === MAIL_139_PROVIDER) {
      expect(within(dialog).getByText(provider.setup!.description)).toHaveTextContent('广电号码暂不支持')
      expect(within(dialog).getByText(provider.setup!.credentialHint)).toHaveTextContent('90 天')
      expect(within(dialog).getByText(provider.setup!.emailHint)).toHaveTextContent('分别修改 IMAP / SMTP 登录用户名')
    }
    await userEvent.click(within(dialog).getByRole('button', { name: '如何获取授权码' }))
    expect(client.openProviderHelp).toHaveBeenCalledWith(provider.id)
    fillQqConnection(dialog, emailAddress)
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    const preference = await within(dialog).findByRole('switch', { name: '使用服务商图标' })
    expect(preference).toBeChecked()
    expect(within(dialog).getByRole('radio', { name: '不标识' })).toHaveAttribute('aria-checked', 'true')
    await userEvent.click(preference)
    expect(within(dialog).getByRole('radio', { name: '不标识' })).toHaveAttribute('aria-checked', 'true')
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveValue(emailAddress)
    expect(within(dialog).getByLabelText('授权码')).toHaveValue('syntheticcodeabc')
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByRole('switch', { name: '使用服务商图标' })).not.toBeChecked()
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount).toHaveBeenCalledOnce()
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject({
      emailAddress, providerId: provider.id, color: null, useProviderIcon: false,
      imap: { host: `imap.${domain}`, port: 993, tls: 'implicit', username: emailAddress },
      smtp: { host: `smtp.${domain}`, port: 465, tls: 'implicit', username: emailAddress },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'syntheticcodeabc' } },
      route: { kind: 'direct' },
    })
  })

  it.each([
    { provider: NETEASE_163_PROVIDER, emailAddress: 'synthetic@126.com' },
    { provider: NETEASE_126_PROVIDER, emailAddress: 'synthetic@163.com' },
    { provider: NETEASE_YEAH_PROVIDER, emailAddress: 'synthetic@163.com' },
    { provider: MAIL_139_PROVIDER, emailAddress: 'synthetic@sub.139.com' },
  ])('keeps an unsupported domain out of $provider.id and clears its entered authorization code', async ({ provider, emailAddress }) => {
    const { client, dialog } = renderDialog()
    await chooseProvider(dialog, provider)
    fillQqConnection(dialog, emailAddress)
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveAttribute('aria-invalid', 'true')
    expect(within(dialog).getByLabelText('授权码')).toHaveValue('')
    expect(within(dialog).queryByLabelText('账号名称')).not.toBeInTheDocument()
    expect(client.createAccount).not.toHaveBeenCalled()
  })

  it('updates the 139 authorization-code guidance when switching languages', async () => {
    const { dialog } = renderDialog()
    await chooseProvider(dialog, MAIL_139_PROVIDER)
    try {
      await act(async () => { await i18n.changeLanguage('en') })
      expect(within(dialog).getByText('Connect 139 Mail')).toBeInTheDocument()
      expect(within(dialog).getByText(MAIL_139_PROVIDER.setup!.description)).toHaveTextContent('China Broadnet numbers currently cannot enable authorization codes')
      expect(within(dialog).getByText(MAIL_139_PROVIDER.setup!.credentialHint)).toHaveTextContent('valid for 90 days')
      expect(within(dialog).getByText(MAIL_139_PROVIDER.setup!.emailHint)).toHaveTextContent('edit the IMAP and SMTP login usernames separately in Advanced settings')
      await act(async () => { await i18n.changeLanguage('zh-CN') })
      expect(within(dialog).getByText('连接 139 邮箱')).toBeInTheDocument()
      expect(within(dialog).getByText(MAIL_139_PROVIDER.setup!.credentialHint)).toHaveTextContent('以网页显示的到期日为准')
    } finally {
      await act(async () => { await i18n.changeLanguage('zh-CN') })
    }
  })

  it('discards a NetEase authorization code and advanced overrides when choosing a different provider', async () => {
    const { client, dialog } = renderDialog()
    await chooseProvider(dialog, NETEASE_163_PROVIDER)
    fillQqConnection(dialog, 'synthetic@163.com')
    await userEvent.click(within(dialog).getByText('高级设置'))
    fireEvent.change(within(dialog).getByLabelText('IMAP 主机'), { target: { value: 'imap.override.example' } })
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '放弃修改' }))
    await chooseProvider(dialog, NETEASE_126_PROVIDER)
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveValue('')
    expect(within(dialog).getByLabelText('授权码')).toHaveValue('')
    await userEvent.click(within(dialog).getByText('高级设置'))
    expect(within(dialog).getByLabelText('IMAP 主机')).toHaveValue('imap.126.com')
    expect(client.createAccount).not.toHaveBeenCalled()
  })

  it.each([NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER, MAIL_139_PROVIDER])('returns a rejected $id credential to an empty field while preserving the address', async (provider) => {
    const client = testClient()
    client.createAccount.mockRejectedValue(new MailxClientError({ code: 'validation', field: 'sharedPassword', message: 'PRIVATE_SYNTHETIC_SERVER_RESPONSE' }))
    const { dialog } = renderDialog(client)
    await chooseProvider(dialog, provider)
    const emailAddress = `synthetic@${provider.preset!.domains[0]}`
    fillQqConnection(dialog, emailAddress)
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    expect(await within(dialog).findByLabelText('授权码')).toHaveValue('')
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveValue(emailAddress)
    expect(within(dialog).getByLabelText('授权码')).toHaveAttribute('aria-invalid', 'true')
    expect(dialog).not.toHaveTextContent('PRIVATE_SYNTHETIC_SERVER_RESPONSE')
    expect(client.createAccount).toHaveBeenCalledOnce()
  })

  it('keeps an opted-out provider icon choice when returning through the connection step', async () => {
    const { client, dialog, onCreated } = renderDialog()
    await chooseQq(dialog)
    fillQqConnection(dialog)
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    const preference = await within(dialog).findByRole('switch', { name: '使用服务商图标' })
    expect(preference).toBeChecked()
    await userEvent.click(preference)
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByRole('switch', { name: '使用服务商图标' })).not.toBeChecked()
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config).toMatchObject({ providerId: 'qq', color: null, useProviderIcon: false })
  })

  it.each([
    { initial: 'manual' as const, initialColor: '绿', next: 'qq' as const, nextColor: '不标识', expectedColor: null },
    { initial: 'qq' as const, initialColor: '不标识', next: 'manual' as const, nextColor: '绿', expectedColor: '#16a34a' },
  ])('updates the untouched color default when switching from $initial to $next', async ({ initial, initialColor, next, nextColor, expectedColor }) => {
    const { client, dialog, onCreated } = renderDialog()
    await toProfile(dialog, initial)
    expect(within(dialog).getByRole('radio', { name: initialColor })).toHaveAttribute('aria-checked', 'true')
    await backToProviders(dialog)
    await toProfile(dialog, next)
    expect(within(dialog).getByRole('radio', { name: nextColor })).toHaveAttribute('aria-checked', 'true')
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config.color).toBe(expectedColor)
  })

  it.each([
    { initial: 'qq' as const, next: 'manual' as const, choice: '紫', expectedColor: '#7c3aed' },
    { initial: 'qq' as const, next: 'manual' as const, choice: '不标识', expectedColor: null },
    { initial: 'manual' as const, next: 'qq' as const, choice: '不标识', expectedColor: null },
  ])('keeps an explicit $choice choice across connection steps and a switch from $initial to $next', async ({ initial, next, choice, expectedColor }) => {
    const { client, dialog, onCreated } = renderDialog()
    await toProfile(dialog, initial)
    await userEvent.click(within(dialog).getByRole('radio', { name: choice }))
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByRole('radio', { name: choice })).toHaveAttribute('aria-checked', 'true')
    await backToProviders(dialog)
    await toProfile(dialog, next)
    expect(within(dialog).getByRole('radio', { name: choice })).toHaveAttribute('aria-checked', 'true')
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    expect(client.createAccount.mock.calls[0][0].config.color).toBe(expectedColor)
  })

  it.each([
    { method: 'qq' as const, choice: '紫', defaultColor: '不标识' },
    { method: 'manual' as const, choice: '不标识', defaultColor: '绿' },
  ])('resets an explicit $choice choice on a fresh $method opening', async ({ method, choice, defaultColor }) => {
    const props = { client: testClient(), accounts: [exampleAccount({ color: '#2563eb' })], onClose: vi.fn(), onCreated: vi.fn() }
    const { rerender } = render(<AddAccountDialog {...props} open />)
    await toProfile(screen.getByRole('dialog'), method)
    await userEvent.click(screen.getByRole('radio', { name: choice }))
    rerender(<AddAccountDialog {...props} open={false} />)
    rerender(<AddAccountDialog {...props} open />)
    await toProfile(screen.getByRole('dialog'), method)
    expect(screen.getByRole('radio', { name: defaultColor })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('radio', { name: choice })).toHaveAttribute('aria-checked', 'false')
  })

  it('retains the QQ address and authorization code when returning from the profile', async () => {
    const { dialog } = renderDialog()
    await chooseQq(dialog)
    fillQqConnection(dialog, 'synthetic@foxmail.com')
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('button', { name: '上一步' }))
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveValue('synthetic@foxmail.com')
    expect(within(dialog).getByLabelText('授权码')).toHaveValue('syntheticcodeabc')
  })

  it('keeps a non-QQ mailbox out of the QQ preset and clears its authorization code after validation', async () => {
    const { client, dialog } = renderDialog()
    await chooseQq(dialog)
    fillQqConnection(dialog, 'synthetic@example.com')
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveAttribute('aria-invalid', 'true')
    expect(within(dialog).getByLabelText('授权码')).toHaveValue('')
    expect(within(dialog).queryByLabelText('账号名称')).not.toBeInTheDocument()
    expect(client.createAccount).not.toHaveBeenCalled()
  })

  it('returns a rejected QQ credential to an empty authorization-code field without losing the address', async () => {
    const client = testClient()
    client.createAccount.mockRejectedValue(new MailxClientError({ code: 'validation', field: 'sharedPassword', message: '凭证无效。' }))
    const { dialog } = renderDialog(client)
    await chooseQq(dialog)
    fillQqConnection(dialog)
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    await within(dialog).findByLabelText('账号名称')
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    expect(await within(dialog).findByLabelText('授权码')).toHaveValue('')
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveValue('synthetic@qq.com')
    expect(within(dialog).getByLabelText('授权码')).toHaveAttribute('aria-invalid', 'true')
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
    expect(await within(dialog).findByText('配置参数无效，请检查标记的字段。')).toBeInTheDocument()
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

  it.each(['紫', '不标识'])('returns to a rejected connection without losing an explicit %s choice', async (choice) => {
    const client = testClient()
    client.createAccount.mockRejectedValue(new MailxClientError({ code: 'validation', field: 'imap.host', message: '无法使用这个主机。' }))
    const { dialog } = renderDialog(client)
    await toProfile(dialog)
    await userEvent.click(within(dialog).getByRole('radio', { name: choice }))
    await userEvent.click(within(dialog).getByRole('button', { name: '完成' }))
    expect(await within(dialog).findByText('请填写主机名或 IP 地址，不要包含协议、路径、用户名或密码。')).toBeInTheDocument()
    expect(within(dialog).getByLabelText('IMAP 主机')).toHaveAttribute('aria-invalid', 'true')
    // The password went out with the failed request and must be entered again.
    expect(within(dialog).getByLabelText('收发密码')).toHaveValue('')
    fillConnection(dialog)
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(await within(dialog).findByRole('radio', { name: choice })).toHaveAttribute('aria-checked', 'true')
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
    expect(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ })).toBeChecked()
    expect(within(dialog).getByRole('button', { name: '下一步' })).toBeEnabled()
  })

  it('clears a hidden choice and keeps next disabled after clearing the search', async () => {
    const { dialog } = renderDialog()
    await userEvent.click(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ }))
    const search = within(dialog).getByRole('searchbox', { name: '搜索邮箱服务商' })
    fireEvent.change(search, { target: { value: 'not-a-provider' } })
    expect(within(dialog).queryByRole('radio')).not.toBeInTheDocument()
    expect(within(dialog).getByRole('status')).toHaveTextContent('没有找到匹配的服务商')
    expect(within(dialog).getByRole('button', { name: '下一步' })).toBeDisabled()
    fireEvent.change(search, { target: { value: '' } })
    expect(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ })).not.toBeChecked()
    expect(within(dialog).getByRole('button', { name: '下一步' })).toBeDisabled()
  })

  it('keeps the query and choice when returning, and resets them on a fresh opening', async () => {
    const client = testClient()
    const props = { client, accounts: [], onClose: vi.fn(), onCreated: vi.fn() }
    const { rerender } = render(<AddAccountDialog {...props} open />)
    const search = screen.getByRole('searchbox', { name: '搜索邮箱服务商' })
    fireEvent.change(search, { target: { value: ' imap ' } })
    await chooseImap(screen.getByRole('dialog'))
    await userEvent.click(screen.getByRole('button', { name: '上一步' }))
    expect(screen.getByRole('searchbox')).toHaveValue(' imap ')
    expect(screen.getByRole('radio', { name: /IMAP\/SMTP/ })).toBeChecked()
    rerender(<AddAccountDialog {...props} open={false} />)
    rerender(<AddAccountDialog {...props} open />)
    expect(screen.getByRole('searchbox')).toHaveValue('')
    expect(screen.getByRole('radio', { name: /IMAP\/SMTP/ })).not.toBeChecked()
  })

  it('opens the connection step by double clicking a provider row', async () => {
    const { dialog } = renderDialog()
    await userEvent.dblClick(within(dialog).getByText('手动配置其他邮箱'))
    expect(within(dialog).getByLabelText('邮箱地址')).toBeInTheDocument()
    expect(within(dialog).getByText('第 2 步，共 3 步')).toBeInTheDocument()
  })

  it('clears a choice hidden by a language change and does not restore it when switching back', async () => {
    const { dialog } = renderDialog()
    await userEvent.click(within(dialog).getByRole('radio', { name: 'IMAP/SMTP' }))
    fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: '手动配置' } })
    expect(within(dialog).getByRole('button', { name: '下一步' })).toBeEnabled()
    try {
      await act(async () => { await i18n.changeLanguage('en') })
      expect(within(dialog).queryByRole('radio')).not.toBeInTheDocument()
      expect(within(dialog).getByRole('button', { name: 'Next' })).toBeDisabled()
      await act(async () => { await i18n.changeLanguage('zh-CN') })
      expect(within(dialog).getByRole('radio', { name: 'IMAP/SMTP' })).not.toBeChecked()
      expect(within(dialog).getByRole('button', { name: '下一步' })).toBeDisabled()
    } finally {
      await act(async () => { await i18n.changeLanguage('zh-CN') })
    }
  })

  it('validates the retained choice when returning after a language change', async () => {
    const { dialog } = renderDialog()
    fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: '手动配置' } })
    await chooseImap(dialog)
    try {
      await act(async () => { await i18n.changeLanguage('en') })
      await userEvent.click(within(dialog).getByRole('button', { name: 'Back' }))
      expect(within(dialog).getByRole('searchbox')).toHaveValue('手动配置')
      expect(within(dialog).queryByRole('radio')).not.toBeInTheDocument()
      expect(within(dialog).getByRole('button', { name: 'Next' })).toBeDisabled()
      await act(async () => { await i18n.changeLanguage('zh-CN') })
      expect(within(dialog).getByRole('radio', { name: 'IMAP/SMTP' })).not.toBeChecked()
    } finally {
      await act(async () => { await i18n.changeLanguage('zh-CN') })
    }
  })

  describe('browser sign-in providers', () => {
    async function toSignInStep(dialog: HTMLElement, provider: MailProvider, address = 'lin@outlook.com') {
      fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: provider.name } })
      await userEvent.click(within(dialog).getByRole('radio', { name: provider.name }))
      await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
      fireEvent.change(within(dialog).getByLabelText('邮箱地址'), { target: { value: address } })
      await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
      return within(dialog).findByRole('button', { name: '在浏览器中登录' })
    }

    it('shows the help under the Microsoft entries', async () => {
      const { dialog } = renderDialog()
      fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: 'outlook' } })
      expect(within(dialog).getByText(/登录前请先在 Outlook 设置中开启 IMAP/)).toBeInTheDocument()
      expect(within(dialog).getByText(/公司自己搭建的 Exchange 服务器/)).toBeInTheDocument()
    })

    it('asks only for the address, never for servers or a password', async () => {
      const { dialog } = renderDialog()
      fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: 'outlook' } })
      await userEvent.click(within(dialog).getByRole('radio', { name: 'Outlook' }))
      await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
      expect(within(dialog).getByLabelText('邮箱地址')).toBeInTheDocument()
      expect(within(dialog).getByText('MailX 会在浏览器中打开微软的登录页面。')).toBeInTheDocument()
      expect(within(dialog).queryByLabelText('IMAP 主机')).not.toBeInTheDocument()
      expect(within(dialog).queryByLabelText('收发密码')).not.toBeInTheDocument()
      expect(within(dialog).queryByRole('button', { name: '测试登录' })).not.toBeInTheDocument()
      // The usual network settings stay available.
      await userEvent.click(within(dialog).getByText('高级设置'))
      expect(within(dialog).getByLabelText('连接方式')).toBeInTheDocument()
    })

    it.each([[MICROSOFT_PERSONAL_PROVIDER, 'lin@outlook.com'], [MICROSOFT_WORK_PROVIDER, 'lin@contoso.com']])('signs in through the browser for %s without creating a password account', async (provider, address) => {
      const { client, dialog, onCreated } = renderDialog()
      const signIn = await toSignInStep(dialog, provider, address)
      expect(client.verifyDraftConnection).not.toHaveBeenCalled()
      expect(within(dialog).queryByRole('button', { name: '完成' })).not.toBeInTheDocument()
      await userEvent.click(signIn)
      await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
      expect(client.createAccount).not.toHaveBeenCalled()
      expect(client.beginOAuthAccount).toHaveBeenCalledOnce()
      const { providerId, request } = client.beginOAuthAccount.mock.calls[0][0]
      expect(providerId).toBe(provider.id)
      expect(request.config).toMatchObject({ emailAddress: address, providerId: provider.id, authentication: { kind: 'oauth2' }, route: { kind: 'direct' } })
      expect(client.finishOAuthAccount).toHaveBeenCalledWith('dddddddd-dddd-4ddd-8ddd-dddddddddddd')
    })

    it('shows a waiting state while the browser is open and cancels the session on request', async () => {
      const client = testClient()
      let release: (reason: unknown) => void = () => undefined
      client.finishOAuthAccount.mockImplementation(() => new Promise((_resolve, reject) => { release = reject }))
      client.cancelOAuthAccount.mockImplementation(async () => { release(new MailxClientError({ code: 'oauthSessionExpired', message: '', field: null })) })
      const { dialog, onCreated } = renderDialog(client)
      await userEvent.click(await toSignInStep(dialog, MICROSOFT_PERSONAL_PROVIDER))
      expect(await within(dialog).findByText('请在浏览器中完成登录')).toBeInTheDocument()
      await userEvent.click(within(dialog).getByRole('button', { name: '取消' }))
      await waitFor(() => expect(client.cancelOAuthAccount).toHaveBeenCalledWith('dddddddd-dddd-4ddd-8ddd-dddddddddddd'))
      await waitFor(() => expect(within(dialog).queryByText('请在浏览器中完成登录')).not.toBeInTheDocument())
      // Canceling is not an error, and the user can try again.
      expect(within(dialog).queryByText('登录没有成功')).not.toBeInTheDocument()
      expect(within(dialog).getByRole('button', { name: '在浏览器中登录' })).toBeEnabled()
      expect(onCreated).not.toHaveBeenCalled()
    })

    it.each([
      ['oauthMailboxRefused', '无法用这个微软账号打开该邮箱'],
      ['oauthUsePersonalEntry', '请改用“Outlook”入口添加'],
      ['oauthUseOrganizationEntry', '请改用“Microsoft 365 / Exchange”入口添加'],
    ] as const)('returns to the address step with a plain message for %s', async (code, text) => {
      const client = testClient()
      client.finishOAuthAccount.mockRejectedValue(new MailxClientError({ code, message: '', field: null }))
      const { dialog, onCreated } = renderDialog(client)
      await userEvent.click(await toSignInStep(dialog, MICROSOFT_PERSONAL_PROVIDER))
      expect(await within(dialog).findByText(new RegExp(text))).toBeInTheDocument()
      expect(within(dialog).getByText('第 2 步，共 3 步')).toBeInTheDocument()
      expect(within(dialog).getByLabelText('邮箱地址')).toBeInTheDocument()
      expect(within(dialog).getByText('登录没有成功')).toBeInTheDocument()
      expect(onCreated).not.toHaveBeenCalled()
    })

    it('stays on the last step when the user did not allow access, so signing in again is one click', async () => {
      const client = testClient()
      client.finishOAuthAccount.mockRejectedValue(new MailxClientError({ code: 'oauthAuthorizationDenied', message: '', field: null }))
      const { dialog } = renderDialog(client)
      await userEvent.click(await toSignInStep(dialog, MICROSOFT_PERSONAL_PROVIDER))
      expect(await within(dialog).findByText(/登录时没有允许 MailX 访问邮箱/)).toBeInTheDocument()
      expect(within(dialog).getByText('第 3 步，共 3 步')).toBeInTheDocument()
      expect(within(dialog).getByRole('button', { name: '在浏览器中登录' })).toBeEnabled()
    })

    it('abandons a pending sign-in when the dialog is closed', async () => {
      const client = testClient()
      client.finishOAuthAccount.mockImplementation(() => new Promise(() => undefined))
      const onClose = vi.fn()
      const props = { client, accounts: [exampleAccount({ color: '#2563eb' })], onClose, onCreated: vi.fn() }
      const view = render(<AddAccountDialog {...props} open />)
      const dialog = screen.getByRole('dialog', { name: '添加邮箱账号' })
      await userEvent.click(await toSignInStep(dialog, MICROSOFT_WORK_PROVIDER, 'lin@contoso.com'))
      await within(dialog).findByText('请在浏览器中完成登录')
      view.rerender(<AddAccountDialog {...props} open={false} />)
      await waitFor(() => expect(client.cancelOAuthAccount).toHaveBeenCalledWith('dddddddd-dddd-4ddd-8ddd-dddddddddddd'))
    })
  })
})
