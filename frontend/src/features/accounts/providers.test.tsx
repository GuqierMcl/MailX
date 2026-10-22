import { render, screen } from '@/test/render'
import { describe, expect, it } from 'vitest'
import { ProviderBadge } from './provider-badge'
import { GENERIC_PROVIDER, providerOf } from './providers'

describe('mail providers', () => {
  it('labels a manually configured account as IMAP/SMTP', () => {
    expect(providerOf(null)).toBe(GENERIC_PROVIDER)
    render(<ProviderBadge providerId={null} />)
    expect(screen.getByText('IMAP/SMTP')).toBeInTheDocument()
  })

  it('still labels a provider this build does not know', () => {
    expect(providerOf('future-mail')).toMatchObject({ id: 'future-mail', name: 'future-mail' })
  })
})
