import { render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { Account } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { exampleAccount, testClient } from './test-fixtures'
import { SignInAgain } from './sign-in-again'

function oauthAccount(state: 'connected' | 'needsReauthorization'): Account {
  const endpoint = { id: 'oauth-profile', generation: 1, credentialConfigured: true, trust: 'trusted' as const }
  return exampleAccount({
    providerId: 'microsoft-work', revision: 4,
    authentication: { kind: 'oauth2', configuration: { providerId: 'microsoft-work', issuer: 'https://issuer.example', clientRegistrationId: 'registration', authorizationSubject: 'subject', targetMailboxResource: null }, state, imap: endpoint, smtp: endpoint },
  })
}

describe('sign in again', () => {
  it('is a prominent warning when the account needs it, and starts the sign-in for the exact revision', async () => {
    const client = testClient()
    const onSignedIn = vi.fn()
    const account = oauthAccount('needsReauthorization')
    render(<SignInAgain client={client} account={account} onSignedIn={onSignedIn} />)
    expect(screen.getByText('需要重新登录，这个邮箱才能继续使用')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重新登录' }))
    await waitFor(() => expect(onSignedIn).toHaveBeenCalledOnce())
    expect(client.reauthorizeOAuthAccount).toHaveBeenCalledWith({ providerId: 'microsoft-work', accountId: account.id, expectedRevision: 4 })
    expect(client.finishOAuthAccount).toHaveBeenCalledWith('dddddddd-dddd-4ddd-8ddd-dddddddddddd')
  })

  it('is a quiet button while the sign-in works', () => {
    render(<SignInAgain client={testClient()} account={oauthAccount('connected')} onSignedIn={vi.fn()} />)
    expect(screen.queryByText('需要重新登录，这个邮箱才能继续使用')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重新登录' })).toBeInTheDocument()
  })

  it('shows a plain message when signing in again fails', async () => {
    const client = testClient()
    client.finishOAuthAccount.mockRejectedValue(new MailxClientError({ code: 'oauthMailboxRefused', message: '', field: null }))
    const onSignedIn = vi.fn()
    render(<SignInAgain client={client} account={oauthAccount('needsReauthorization')} onSignedIn={onSignedIn} />)
    await userEvent.click(screen.getByRole('button', { name: '重新登录' }))
    expect(await screen.findByText('重新登录没有成功')).toBeInTheDocument()
    expect(screen.getByText(/无法用这个微软账号打开该邮箱/)).toBeInTheDocument()
    expect(onSignedIn).not.toHaveBeenCalled()
  })

  it('cancels a pending sign-in and does not report it as a failure', async () => {
    const client = testClient()
    let release: (reason: unknown) => void = () => undefined
    client.finishOAuthAccount.mockImplementation(() => new Promise((_resolve, reject) => { release = reject }))
    client.cancelOAuthAccount.mockImplementation(async () => { release(new MailxClientError({ code: 'oauthSessionExpired', message: '', field: null })) })
    render(<SignInAgain client={client} account={oauthAccount('needsReauthorization')} onSignedIn={vi.fn()} />)
    await userEvent.click(screen.getByRole('button', { name: '重新登录' }))
    expect(await screen.findByText('请在浏览器中完成登录')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(client.cancelOAuthAccount).toHaveBeenCalledOnce())
    await waitFor(() => expect(screen.queryByText('请在浏览器中完成登录')).not.toBeInTheDocument())
    expect(screen.queryByText('重新登录没有成功')).not.toBeInTheDocument()
  })

  it('renders nothing for a password account', () => {
    const { container } = render(<SignInAgain client={testClient()} account={exampleAccount()} onSignedIn={vi.fn()} />)
    expect(container).toBeEmptyDOMElement()
  })
})
