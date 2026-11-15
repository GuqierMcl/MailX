import { render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Account } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { toast } from '@/components/ui/toast'
import { exampleAccount, testClient } from './test-fixtures'
import { SignInAgain } from './sign-in-again'

function oauthAccount(state: 'connected' | 'needsReauthorization'): Account {
  const endpoint = { id: 'oauth-profile', generation: 1, credentialConfigured: true, trust: 'trusted' as const }
  return exampleAccount({
    providerId: 'microsoft-work', revision: 4,
    authentication: { kind: 'oauth2', configuration: { providerId: 'microsoft-work', issuer: 'https://issuer.example', clientRegistrationId: 'registration', authorizationSubject: 'subject', targetMailboxResource: null }, state, imap: endpoint, smtp: endpoint },
  })
}

vi.mock('@/components/ui/toast', () => ({ toast: { info: vi.fn(() => 'waiting-toast'), success: vi.fn(), error: vi.fn(), close: vi.fn() } }))

describe('sign in again', () => {
  beforeEach(() => vi.clearAllMocks())

  it('is the primary button when the account needs it, and starts the sign-in for the exact revision', async () => {
    const client = testClient()
    const onSignedIn = vi.fn()
    const account = oauthAccount('needsReauthorization')
    render(<SignInAgain client={client} account={account} onSignedIn={onSignedIn} />)
    // The page header says what is wrong; this component is only the button.
    expect(screen.queryByText('需要重新登录，这个邮箱才能继续使用')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重新登录' }))
    await waitFor(() => expect(onSignedIn).toHaveBeenCalledOnce())
    expect(toast.success).toHaveBeenCalledWith('已重新登录')
    expect(client.reauthorizeOAuthAccount).toHaveBeenCalledWith({ providerId: 'microsoft-work', accountId: account.id, expectedRevision: 4 })
    expect(client.finishOAuthReauthorization).toHaveBeenCalledWith('dddddddd-dddd-4ddd-8ddd-dddddddddddd')
    // A sign-in again never goes through the new-account calls.
    expect(client.finishOAuthAccount).not.toHaveBeenCalled()
    expect(client.createOAuthAccount).not.toHaveBeenCalled()
  })

  it('is a quiet button while the sign-in works and the primary one when it is needed', () => {
    const { rerender } = render(<SignInAgain client={testClient()} account={oauthAccount('connected')} onSignedIn={vi.fn()} />)
    expect(screen.getByRole('button', { name: '重新登录' })).toHaveClass('border-border')
    rerender(<SignInAgain client={testClient()} account={oauthAccount('connected')} required onSignedIn={vi.fn()} />)
    expect(screen.getByRole('button', { name: '重新登录' })).toHaveClass('bg-primary')
  })

  it('reports a failed sign-in as a plain toast', async () => {
    const client = testClient()
    client.finishOAuthReauthorization.mockRejectedValue(new MailxClientError({ code: 'oauthMailboxRefused', message: '', field: null }))
    const onSignedIn = vi.fn()
    render(<SignInAgain client={client} account={oauthAccount('needsReauthorization')} onSignedIn={onSignedIn} />)
    await userEvent.click(screen.getByRole('button', { name: '重新登录' }))
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith('重新登录没有成功', { description: expect.stringMatching(/无法用这个微软账号打开该邮箱/) }))
    expect(onSignedIn).not.toHaveBeenCalled()
  })

  it('cancels a pending sign-in and does not report it as a failure', async () => {
    const client = testClient()
    let release: (reason: unknown) => void = () => undefined
    client.finishOAuthReauthorization.mockImplementation(() => new Promise((_resolve, reject) => { release = reject }))
    client.cancelOAuthAccount.mockImplementation(async () => { release(new MailxClientError({ code: 'oauthSessionExpired', message: '', field: null })) })
    render(<SignInAgain client={client} account={oauthAccount('needsReauthorization')} onSignedIn={vi.fn()} />)
    await userEvent.click(screen.getByRole('button', { name: '重新登录' }))
    await userEvent.click(await screen.findByRole('button', { name: '取消' }))
    expect(toast.info).toHaveBeenCalledWith('请在浏览器中完成登录', expect.objectContaining({ description: expect.any(String) }))
    await waitFor(() => expect(client.cancelOAuthAccount).toHaveBeenCalledOnce())
    await waitFor(() => expect(toast.close).toHaveBeenCalledWith('waiting-toast'))
    expect(await screen.findByRole('button', { name: '重新登录' })).toBeInTheDocument()
    expect(toast.error).not.toHaveBeenCalled()
  })

  it('renders nothing for a password account', () => {
    const { container } = render(<SignInAgain client={testClient()} account={exampleAccount()} onSignedIn={vi.fn()} />)
    expect(container).toBeEmptyDOMElement()
  })
})
