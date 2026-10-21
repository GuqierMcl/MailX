import { render, screen, waitFor } from '@/test/render'
import { describe, expect, it } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { avatarHue } from './format'
import { AvatarSourceProvider, SenderAvatar } from './sender-avatar'

const sender = { name: 'Lin', address: 'Lin@Example.com' }

function setup(enabled: boolean) {
  const client = testClient()
  client.getAvatar.mockResolvedValue({ mimeType: 'image/png', data: 'AAAA' })
  render(
    <AvatarSourceProvider client={client} enabled={enabled}>
      <SenderAvatar sender={sender} accountId="account-1" />
    </AvatarSourceProvider>,
  )
  return client
}

describe('sender avatar', () => {
  it('shows a stable colored initial and never asks for an avatar unless enabled', async () => {
    const client = setup(false)
    const initial = screen.getByText('L')
    expect(initial).toHaveStyle({ backgroundColor: `hsl(${avatarHue('lin@example.com')} 45% 42%)` })
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(client.getAvatar).not.toHaveBeenCalled()
  })

  it('asks once with the normalized address when enabled and keeps the initial as the fallback', async () => {
    const client = setup(true)
    await waitFor(() => expect(client.getAvatar).toHaveBeenCalledWith('account-1', 'lin@example.com'))
    expect(client.getAvatar).toHaveBeenCalledTimes(1)
    expect(screen.getByText('L')).toBeInTheDocument()
  })

  it('derives the same hue for any casing of an address and spreads different ones', () => {
    expect(avatarHue(' A@B.com ')).toBe(avatarHue('a@b.com'))
    expect(new Set(['a@x.io', 'b@x.io', 'c@x.io', 'd@x.io', 'e@x.io'].map(avatarHue)).size).toBeGreaterThan(2)
    expect(avatarHue('a@b.com')).toBeLessThan(360)
  })
})
