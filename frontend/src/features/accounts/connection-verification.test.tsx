import { act, render, screen, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { AccountConnectionVerification } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { ConnectionVerification } from './connection-verification'
import { exampleAccount, testClient } from './test-fixtures'

function report(accountId: string, revision: number): AccountConnectionVerification {
  return {
    accountId, revision,
    imap: { status: 'verified', stage: 'complete', code: null },
    smtp: { status: 'verified', stage: 'complete', code: null },
  }
}

describe('account connection verification', () => {
  it('reports login verification separately from actual email delivery', async () => {
    const account = exampleAccount({ providerId: 'qq' })
    const client = testClient([account])
    render(<ConnectionVerification account={account} client={client} />)
    expect(screen.getByText('验证已保存的连接和登录，不发送邮件。')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('登录验证通过，尚未测试实际投递。')).toBeInTheDocument()
    const status = within(screen.getByRole('status'))
    expect(status.getByText('IMAP 收信')).toBeInTheDocument()
    expect(status.getByText('SMTP 发信')).toBeInTheDocument()
    expect(status.getAllByText('登录成功')).toHaveLength(2)
    expect(client.verifyAccountConnection).toHaveBeenCalledWith(account.id, account.revision, expect.any(AbortSignal))
    expect(client.queueSend).not.toHaveBeenCalled()
  })

  it('keeps IMAP success distinct from SMTP authentication failure and offers QQ authorization help', async () => {
    const account = exampleAccount({ providerId: 'qq' })
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue({
      ...report(account.id, account.revision),
      smtp: { status: 'failed', stage: 'authentication', code: 'authentication' },
    })
    render(<ConnectionVerification account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('登录验证未通过')).toBeInTheDocument()
    expect(within(screen.getByRole('status')).getAllByText('登录成功')).toHaveLength(1)
    expect(screen.getByText(/请确认已开启 IMAP\/SMTP/)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '如何获取授权码' }))
    expect(client.openProviderHelp).toHaveBeenCalledWith('qq')
    expect(screen.queryByText('登录验证通过，尚未测试实际投递。')).not.toBeInTheDocument()
  })

  it('cancels the request and ignores a successful reply arriving after cancellation', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    let resolve!: (value: AccountConnectionVerification) => void
    client.verifyAccountConnection.mockImplementation(() => new Promise((done) => { resolve = done }))
    render(<ConnectionVerification account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    const signal = client.verifyAccountConnection.mock.calls[0][2]
    expect(signal?.aborted).toBe(false)
    await userEvent.click(screen.getByRole('button', { name: '取消验证' }))
    expect(signal?.aborted).toBe(true)
    expect(screen.getByRole('status')).toHaveTextContent('验证已取消')
    await act(async () => { resolve(report(account.id, account.revision)) })
    expect(screen.getByRole('status')).toHaveTextContent('验证已取消')
    expect(screen.queryByText('登录验证通过，尚未测试实际投递。')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '验证收发连接' })).toBeEnabled()
  })

  it('cancels the old revision and ignores its reply when the saved configuration changes', async () => {
    const account = exampleAccount()
    const next = { ...account, revision: account.revision + 1 }
    const client = testClient([account])
    let resolve!: (value: AccountConnectionVerification) => void
    client.verifyAccountConnection.mockImplementationOnce(() => new Promise((done) => { resolve = done }))
    const { rerender } = render(<ConnectionVerification account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    const signal = client.verifyAccountConnection.mock.calls[0][2]
    rerender(<ConnectionVerification account={next} client={client} />)
    expect(signal?.aborted).toBe(true)
    await act(async () => { resolve(report(account.id, account.revision)) })
    expect(screen.queryByText('登录验证通过，尚未测试实际投递。')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('登录验证通过，尚未测试实际投递。')).toBeInTheDocument()
    expect(client.verifyAccountConnection.mock.calls[1]).toEqual([next.id, next.revision, expect.any(AbortSignal)])
  })

  it('cancels an in-flight request when the connection view is closed', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockImplementation(() => new Promise(() => undefined))
    const { unmount } = render(<ConnectionVerification account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    const signal = client.verifyAccountConnection.mock.calls[0][2]
    unmount()
    expect(signal?.aborted).toBe(true)
  })

  it('refuses a report for a different saved configuration', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockResolvedValue(report(account.id, account.revision + 1))
    render(<ConnectionVerification account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByText('配置已改变，请重新验证当前连接。')).toBeInTheDocument()
    expect(screen.queryByText('登录验证通过，尚未测试实际投递。')).not.toBeInTheDocument()
  })

  it('does not render raw failure messages returned by the transport', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    client.verifyAccountConnection.mockRejectedValue(new MailxClientError({ code: 'internal', field: null, message: 'PRIVATE_SYNTHETIC_SERVER_RESPONSE' }))
    render(<ConnectionVerification account={account} client={client} />)
    await userEvent.click(screen.getByRole('button', { name: '验证收发连接' }))
    expect(await screen.findByRole('status')).not.toHaveTextContent('PRIVATE_SYNTHETIC_SERVER_RESPONSE')
    expect(screen.getByRole('status')).not.toBeEmptyDOMElement()
    expect(screen.queryByText('登录验证通过，尚未测试实际投递。')).not.toBeInTheDocument()
  })
})
