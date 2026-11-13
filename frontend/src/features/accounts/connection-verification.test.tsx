import { act, render, screen } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { AccountConnectionVerification } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import type { Account } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { ConnectionVerificationActions, ProtocolOutcome, VerificationHelp, VerificationNotice, VerificationSummary } from './connection-verification'
import { useConnectionVerification, verificationReport } from './use-connection-verification'
import { exampleAccount, testClient } from './test-fixtures'
import { ICLOUD_PROVIDER, MAIL_139_PROVIDER, NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER, QQ_PROVIDER } from './providers'

function report(accountId: string, revision: number): AccountConnectionVerification {
  return {
    accountId, revision,
    imap: { status: 'verified', stage: 'complete', code: null },
    smtp: { status: 'verified', stage: 'complete', code: null },
  }
}

/** What the account page spreads over its footer, method row and details, in one place. */
function Harness({ account, client }: { account: Account; client: MailxClient }) {
  const verification = useConnectionVerification(account, client)
  return <>
    <ConnectionVerificationActions verification={verification} client={client} />
    <VerificationSummary state={verification.state} />
    <VerificationNotice state={verification.state} />
    <ProtocolOutcome state={verification.state} protocol="imap" />
    <ProtocolOutcome state={verification.state} protocol="smtp" />
    <VerificationHelp report={verificationReport(verification.state)} client={client} providerId={account.providerId} />
  </>
}

describe('account connection verification', () => {
  it('shows a short pass without implying that mail is delivered', async () => {
    const account = exampleAccount({ providerId: 'qq' })
    const client = testClient([account])
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('· 收发均已通过')).toBeInTheDocument()
    expect(screen.getAllByText('已通过')).toHaveLength(2)
    expect(client.verifyAccountConnection).toHaveBeenCalledWith(account.id, account.revision, expect.any(AbortSignal))
    expect(client.queueSend).not.toHaveBeenCalled()
  })

  it('names the side that failed, with the reason in small red text and passed sides kept quiet', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue({
      ...report(account.id, account.revision),
      imap: { status: 'failed', stage: 'connection', code: 'configuration' },
      smtp: { status: 'failed', stage: 'connection', code: 'secretStore' },
    })
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('· 收发均未通过')).toHaveClass('text-xs', 'text-destructive')
    for (const reason of ['连接配置不可用，请修改连接后重试。', '系统凭据库暂不可用，请解锁后重试。']) {
      expect(screen.getByText(reason)).toHaveClass('text-xs', 'text-destructive')
    }
    expect(screen.queryByText('已通过')).not.toBeInTheDocument()
  })

  it.each([QQ_PROVIDER, NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER, MAIL_139_PROVIDER])('keeps IMAP success distinct from SMTP authentication failure and offers $id authorization help', async (provider) => {
    const account = exampleAccount({ providerId: provider.id })
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue({
      ...report(account.id, account.revision),
      smtp: { status: 'failed', stage: 'authentication', code: 'authentication' },
    })
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText(/^· (收信|发信)未通过$/)).toBeInTheDocument()
    expect(screen.getAllByText('已通过')).toHaveLength(1)
    expect(screen.getByText(provider.setup!.authenticationHelp)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '如何获取授权码' }))
    expect(client.openProviderHelp).toHaveBeenCalledWith(provider.id)
    expect(screen.queryByText('· 收发均已通过')).not.toBeInTheDocument()
  })

  it('offers iCloud app-password recovery when IMAP authentication fails without retrying or sending', async () => {
    const account = exampleAccount({ providerId: 'icloud' })
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue({
      ...report(account.id, account.revision),
      imap: { status: 'failed', stage: 'authentication', code: 'authentication' },
    })
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText(/^· (收信|发信)未通过$/)).toBeInTheDocument()
    expect(screen.getAllByText('已通过')).toHaveLength(1)
    expect(screen.getByText(ICLOUD_PROVIDER.setup!.authenticationHelp)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '如何获取授权码' })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '如何生成应用专用密码' }))
    expect(client.openProviderHelp).toHaveBeenCalledWith('icloud')
    expect(client.verifyAccountConnection).toHaveBeenCalledOnce()
    expect(client.queueSend).not.toHaveBeenCalled()
  })

  it.each([NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER])('explains a $id mailbox policy refusal separately from a rejected authorization code', async (provider) => {
    const account = exampleAccount({ providerId: provider.id })
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue({
      ...report(account.id, account.revision),
      imap: { status: 'failed', stage: 'mailbox', code: 'providerPolicy' },
    })
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText(/^· (收信|发信)未通过$/)).toBeInTheDocument()
    expect(screen.getAllByText('已通过')).toHaveLength(1)
    expect(screen.getByText(provider.setup!.providerPolicyHelp!)).toBeInTheDocument()
    expect(screen.queryByText(provider.setup!.authenticationHelp)).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '如何获取授权码' })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '查看官方帮助' }))
    expect(client.openProviderHelp).toHaveBeenCalledWith(provider.id)
    expect(client.verifyAccountConnection).toHaveBeenCalledOnce()
    expect(client.queueSend).not.toHaveBeenCalled()
  })

  it('keeps a provider policy failure actionable without guessing a provider for a manual account', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue({
      ...report(account.id, account.revision),
      imap: { status: 'failed', stage: 'mailbox', code: 'providerPolicy' },
    })
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('· 收信未通过')).toBeInTheDocument()
    expect(screen.getByText('已通过')).toBeInTheDocument()
    expect(screen.getByText(/服务商/)).toHaveClass('text-destructive')
    expect(screen.queryByRole('button', { name: '查看官方帮助' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '如何获取授权码' })).not.toBeInTheDocument()
    expect(client.openProviderHelp).not.toHaveBeenCalled()
  })

  it('prioritizes provider policy guidance when the other protocol also rejects authentication', async () => {
    const provider = NETEASE_163_PROVIDER
    const account = exampleAccount({ providerId: provider.id })
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue({
      ...report(account.id, account.revision),
      imap: { status: 'failed', stage: 'mailbox', code: 'providerPolicy' },
      smtp: { status: 'failed', stage: 'authentication', code: 'authentication' },
    })
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText(provider.setup!.providerPolicyHelp!)).toBeInTheDocument()
    expect(screen.queryByText(provider.setup!.authenticationHelp)).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '如何获取授权码' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '查看官方帮助' })).toBeInTheDocument()
  })

  it('cancels the request and ignores a successful reply arriving after cancellation', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    let resolve!: (value: AccountConnectionVerification) => void
    client.verifyAccountConnection.mockImplementation(() => new Promise((done) => { resolve = done }))
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    const signal = client.verifyAccountConnection.mock.calls[0][2]
    expect(signal?.aborted).toBe(false)
    await userEvent.click(screen.getByRole('button', { name: '取消验证' }))
    expect(signal?.aborted).toBe(true)
    expect(screen.getByRole('status')).toHaveTextContent('验证已取消')
    await act(async () => { resolve(report(account.id, account.revision)) })
    expect(screen.getByRole('status')).toHaveTextContent('验证已取消')
    expect(screen.queryByText('· 收发均已通过')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '验证收发连接' })).toBeEnabled()
  })

  it('cancels the old revision and ignores its reply when the saved configuration changes', async () => {
    const account = exampleAccount()
    const next = { ...account, revision: account.revision + 1 }
    const client = testClient([account])
    let resolve!: (value: AccountConnectionVerification) => void
    client.verifyAccountConnection.mockImplementationOnce(() => new Promise((done) => { resolve = done }))
    const { rerender } = render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    const signal = client.verifyAccountConnection.mock.calls[0][2]
    rerender(<Harness account={next} client={client} />)
    expect(signal?.aborted).toBe(true)
    await act(async () => { resolve(report(account.id, account.revision)) })
    expect(screen.queryByText('· 收发均已通过')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('· 收发均已通过')).toBeInTheDocument()
    expect(client.verifyAccountConnection.mock.calls[1]).toEqual([next.id, next.revision, expect.any(AbortSignal)])
  })

  it('cancels an in-flight request when the connection view is closed', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockImplementation(() => new Promise(() => undefined))
    const { unmount } = render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    const signal = client.verifyAccountConnection.mock.calls[0][2]
    unmount()
    expect(signal?.aborted).toBe(true)
  })

  it('refuses a report for a different saved configuration', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue(report(account.id, account.revision + 1))
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('配置已改变，请重新验证当前连接。')).toBeInTheDocument()
    expect(screen.queryByText('· 收发均已通过')).not.toBeInTheDocument()
  })

  it('does not render raw failure messages returned by the transport', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockRejectedValue(new MailxClientError({ code: 'internal', field: null, message: 'PRIVATE_SYNTHETIC_SERVER_RESPONSE' }))
    render(<Harness account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByRole('status')).not.toHaveTextContent('PRIVATE_SYNTHETIC_SERVER_RESPONSE')
    expect(screen.getByRole('status')).not.toBeEmptyDOMElement()
    expect(screen.queryByText('· 收发均已通过')).not.toBeInTheDocument()
  })
})
