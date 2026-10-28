import { render } from '@/test/render'
import { describe, expect, it } from 'vitest'
import { AccountAvatar } from './account-avatar'
import { NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER, QQ_PROVIDER } from './providers'

describe('account avatar', () => {
  it('defaults a known provider to its bundled logo while retaining the account color and requested size', () => {
    const { container } = render(<AccountAvatar displayName="工作邮箱" color="#2563eb" providerId="qq" className="size-[18px] text-[10px]" />)
    const avatar = container.querySelector('[data-slot="account-avatar"]')!
    const logo = avatar.querySelector('img')!
    expect(logo).toHaveAttribute('src', QQ_PROVIDER.logo)
    expect(logo).toHaveAttribute('alt', '')
    expect(logo).toHaveAttribute('aria-hidden', 'true')
    expect(avatar).toHaveAttribute('aria-hidden', 'true')
    expect(avatar).toHaveStyle({ backgroundColor: '#2563eb' })
    expect(avatar).toHaveClass('size-[18px]')
    expect(avatar).not.toHaveTextContent('工')
  })

  it('uses the original colored initial when the account disables its provider icon', () => {
    const { container } = render(<AccountAvatar displayName=" mail" color="#16a34a" providerId="qq" useProviderIcon={false} />)
    const avatar = container.querySelector('[data-slot="account-avatar"]')!
    expect(avatar.querySelector('img')).toBeNull()
    expect(avatar).toHaveTextContent(/^M$/)
    expect(avatar).toHaveStyle({ backgroundColor: '#16a34a' })
  })

  it.each([NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER])('reuses the account icon preference for $id without a network logo lookup', (provider) => {
    const { container, rerender } = render(<AccountAvatar displayName="网易邮箱" color="#2563eb" providerId={provider.id} />)
    const logo = container.querySelector('img')!
    expect(logo).toHaveAttribute('src', provider.logo)
    expect(logo.getAttribute('src')).not.toMatch(/^https?:/)
    expect(logo).toHaveAttribute('alt', '')
    rerender(<AccountAvatar displayName="网易邮箱" color="#2563eb" providerId={provider.id} useProviderIcon={false} />)
    expect(container.querySelector('img')).toBeNull()
    expect(container.querySelector('[data-slot="account-avatar"]')).toHaveTextContent(/^网$/)
  })

  it.each([undefined, null, 'future-provider'])('falls back to the neutral initial for provider %s without a bundled logo', (providerId) => {
    const { container } = render(<AccountAvatar displayName="个人邮箱" color={null} providerId={providerId} />)
    const avatar = container.querySelector('[data-slot="account-avatar"]')!
    expect(avatar.querySelector('img, svg')).toBeNull()
    expect(avatar).toHaveTextContent(/^个$/)
    expect(avatar).toHaveClass('bg-muted', 'text-muted-foreground')
    expect(avatar).not.toHaveAttribute('style')
  })

  it('keeps preferences independent for two accounts from the same provider', () => {
    const { container, rerender } = render(<>
      <AccountAvatar displayName="工作邮箱" color="#2563eb" providerId="qq" useProviderIcon />
      <AccountAvatar displayName="个人邮箱" color="#16a34a" providerId="qq" useProviderIcon={false} />
    </>)
    const avatars = () => container.querySelectorAll('[data-slot="account-avatar"]')
    expect(avatars()[0].querySelector('img')).not.toBeNull()
    expect(avatars()[1]).toHaveTextContent(/^个$/)
    rerender(<>
      <AccountAvatar displayName="工作邮箱" color="#2563eb" providerId="qq" useProviderIcon={false} />
      <AccountAvatar displayName="个人邮箱" color="#16a34a" providerId="qq" useProviderIcon={false} />
    </>)
    expect(avatars()[0]).toHaveTextContent(/^工$/)
    expect(avatars()[1]).toHaveTextContent(/^个$/)
    expect(container.querySelector('img')).toBeNull()
  })
})
