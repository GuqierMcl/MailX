import { render, screen } from '@/test/render'
import { describe, expect, it } from 'vitest'
import { ProviderBadge, ProviderIcon } from './provider-badge'
import { filterProviders, GENERIC_PROVIDER, PROVIDERS, providerOf, QQ_PROVIDER } from './providers'

describe('mail providers', () => {
  it('labels a manually configured account as IMAP/SMTP', () => {
    expect(providerOf(null)).toBe(GENERIC_PROVIDER)
    render(<ProviderBadge providerId={null} />)
    expect(screen.getByText('IMAP/SMTP')).toBeInTheDocument()
  })

  it('still labels a provider this build does not know', () => {
    expect(providerOf('future-mail')).toMatchObject({ id: 'future-mail', name: 'future-mail' })
  })

  it.each(['qq', 'QQ.COM', 'foxmail', 'FOXMAIL.COM', '腾讯邮箱'])('finds the shared QQ/Foxmail entry when searching for %s', (query) => {
    expect(filterProviders(PROVIDERS, query)).toEqual([QQ_PROVIDER])
    expect(providerOf('qq')).toBe(QQ_PROVIDER)
  })

  it('uses a bundled logo when the provider supplies one', () => {
    const { container } = render(<ProviderIcon provider={{ ...GENERIC_PROVIDER, logo: '/bundled-provider.svg' }} />)
    expect(container.querySelector('img')).toHaveAttribute('src', '/bundled-provider.svg')
    expect(container.querySelector('img')).toHaveAttribute('alt', '')
    expect(container.querySelector('svg')).not.toBeInTheDocument()
  })
})
