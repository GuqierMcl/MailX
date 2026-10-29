import { render, screen } from '@/test/render'
import { describe, expect, it } from 'vitest'
import { ProviderBadge, ProviderIcon } from './provider-badge'
import { filterProviders, GENERIC_PROVIDER, MAIL_139_PROVIDER, NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER, PROVIDERS, providerOf, QQ_PROVIDER } from './providers'

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

  it.each([
    ['163', NETEASE_163_PROVIDER], ['163.COM', NETEASE_163_PROVIDER],
    ['126', NETEASE_126_PROVIDER], ['126.COM', NETEASE_126_PROVIDER],
    ['yeah', NETEASE_YEAH_PROVIDER], ['YEAH.NET', NETEASE_YEAH_PROVIDER],
  ])('finds the distinct NetEase mailbox for %s', (query, provider) => {
    expect(filterProviders(PROVIDERS, query)).toEqual([provider])
    expect(providerOf(provider.id)).toBe(provider)
    render(<ProviderBadge providerId={provider.id} />)
    expect(screen.getByText(provider.name)).toBeInTheDocument()
  })

  it('finds all NetEase mailboxes by their shared name without including QQ or manual setup', () => {
    expect(filterProviders(PROVIDERS, '网易')).toEqual([NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER])
  })

  it.each(['139', '139.COM', '中国移动', '移动邮箱', 'China Mobile', 'CMCC'])('finds the personal 139 entry for %s', (query) => {
    expect(filterProviders(PROVIDERS, query)).toEqual([MAIL_139_PROVIDER])
    expect(providerOf('139')).toBe(MAIL_139_PROVIDER)
  })

  it('uses reviewed TLS endpoints and a bundled 139 mark', () => {
    expect(MAIL_139_PROVIDER.preset).toEqual({
      domains: ['139.com'],
      imap: { host: 'imap.139.com', port: 993, tls: 'implicit' },
      smtp: { host: 'smtp.139.com', port: 465, tls: 'implicit' },
    })
    const { container } = render(<><ProviderIcon provider={MAIL_139_PROVIDER} /><ProviderBadge providerId="139" /></>)
    const images = container.querySelectorAll('img')
    expect(images).toHaveLength(2)
    for (const image of images) {
      expect(image).toHaveAttribute('src', MAIL_139_PROVIDER.logo)
      expect(image.getAttribute('src')).not.toMatch(/^https?:/)
    }
    expect(screen.getByText('139 邮箱')).toBeInTheDocument()
  })

  it('gives 163, 126 and yeah.net distinct bundled logos in the provider list', () => {
    const providers = [NETEASE_163_PROVIDER, NETEASE_126_PROVIDER, NETEASE_YEAH_PROVIDER]
    const { container } = render(<>{providers.map((provider) => <ProviderIcon key={provider.id} provider={provider} />)}</>)
    const sources = [...container.querySelectorAll('img')].map((image) => image.getAttribute('src'))
    expect(sources).toHaveLength(3)
    expect(sources.every((source) => source && !/^https?:/.test(source))).toBe(true)
    expect(new Set(sources).size).toBe(3)
  })

  it('uses a bundled logo when the provider supplies one', () => {
    const { container } = render(<ProviderIcon provider={{ ...GENERIC_PROVIDER, logo: '/bundled-provider.svg' }} />)
    expect(container.querySelector('img')).toHaveAttribute('src', '/bundled-provider.svg')
    expect(container.querySelector('img')).toHaveAttribute('alt', '')
    expect(container.querySelector('svg')).not.toBeInTheDocument()
  })
})
