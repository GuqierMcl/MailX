import { render, screen } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { ProviderAuthHelp } from './provider-auth-help'
import { ICLOUD_PROVIDER, MAIL_139_PROVIDER, NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER } from './providers'
import { testClient } from './test-fixtures'

describe('provider authorization help', () => {
  it.each([NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER, MAIL_139_PROVIDER])('reports and recovers from opening $id help without exposing a transport error', async (provider) => {
    const client = testClient()
    client.openProviderHelp.mockRejectedValueOnce(new Error('PRIVATE_SYNTHETIC_SERVER_RESPONSE'))
    render(<ProviderAuthHelp client={client} providerId={provider.id} />)
    const help = screen.getByRole('button', { name: '如何获取授权码' })
    await userEvent.click(help)
    expect(await screen.findByRole('alert')).not.toBeEmptyDOMElement()
    expect(screen.queryByText('PRIVATE_SYNTHETIC_SERVER_RESPONSE')).not.toBeInTheDocument()
    await userEvent.click(help)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(client.openProviderHelp.mock.calls).toEqual([[provider.id], [provider.id]])
  })

  it.each([null, 'future-provider'])('omits provider-specific authorization guidance for %s', (providerId) => {
    const { container } = render(<ProviderAuthHelp client={testClient()} providerId={providerId} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('explains iCloud app-password revocation and recovers a failed official help link', async () => {
    const client = testClient()
    client.openProviderHelp.mockRejectedValueOnce(new Error('PRIVATE_SYNTHETIC_SERVER_RESPONSE'))
    render(<ProviderAuthHelp client={client} providerId="icloud" />)
    expect(screen.getByText(ICLOUD_PROVIDER.setup!.authenticationHelp)).toHaveTextContent('撤销全部应用专用密码')
    expect(screen.queryByRole('button', { name: '如何获取授权码' })).not.toBeInTheDocument()
    const help = screen.getByRole('button', { name: '如何生成应用专用密码' })
    await userEvent.click(help)
    expect(await screen.findByRole('alert')).not.toHaveTextContent('PRIVATE_SYNTHETIC_SERVER_RESPONSE')
    await userEvent.click(help)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(client.openProviderHelp.mock.calls).toEqual([['icloud'], ['icloud']])
  })
})
