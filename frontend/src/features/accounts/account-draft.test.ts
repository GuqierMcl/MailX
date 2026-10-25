import { describe, expect, it } from 'vitest'
import { changeDraft, draftToConnection, initialDraft } from './account-draft'
import { exampleAccount } from './test-fixtures'

describe('network route input boundary', () => {
  it.each(['socks5', 'httpConnect'] as const)('keeps %s route and proxy DNS explicit while replacing a proxy password', (proxyKind) => {
    const account = exampleAccount()
    const draft = initialDraft(account)
    Object.assign(draft, { routeKind: 'custom', proxyKind, proxyHost: 'proxy.example.com', proxyPort: '8080', proxyAuth: true, proxyUsername: 'proxy-user', proxyPassword: { replace: true, value: 'synthetic-proxy-secret' } })
    expect(draftToConnection(draft, account).route).toEqual({ kind: 'custom', proxyKind, host: 'proxy.example.com', port: 8080, dns: 'proxy', imageProxyDns: false, confirmImageProxyDns: false, authentication: { username: 'proxy-user', password: { action: 'replace', value: 'synthetic-proxy-secret' } } })
  })

  it('refuses userinfo in a proxy endpoint before sending anything to the host', () => {
    const account = exampleAccount()
    const draft = initialDraft(account)
    Object.assign(draft, { routeKind: 'custom', proxyHost: 'user:secret@proxy.example.com' })
    expect(() => draftToConnection(draft, account)).toThrow('请填写主机名或 IP 地址')
  })

  it('hydrates configured proxy authentication as keep without a password value', () => {
    const account = exampleAccount({ route: { kind: 'custom', proxyKind: 'httpConnect', host: 'proxy.example.com', port: 8080, dns: 'proxy', imageProxyDns: false, authentication: { username: 'user', passwordConfigured: true, trust: 'trusted' } } })
    const draft = initialDraft(account)
    expect(draft.proxyPassword).toEqual({ replace: false, value: '' })
    expect(draftToConnection(draft, account).route).toMatchObject({ authentication: { username: 'user', password: { action: 'keep' } } })
    draft.routeKind = 'inherit'
    expect(draftToConnection(draft, account).route).toEqual({ kind: 'inherit' })
  })

  it('preserves stored consent but requires a new selection after a proxy identity change', () => {
    const account = exampleAccount({ route: { kind: 'custom', proxyKind: 'socks5', host: 'proxy.example.com', port: 1080, dns: 'proxy', imageProxyDns: true, authentication: null } })
    const saved = initialDraft(account)
    expect(draftToConnection(saved, account).route).toMatchObject({ imageProxyDns: true, confirmImageProxyDns: false })
    let changed = changeDraft(saved, 'proxyHost', 'other.example.com')
    expect(draftToConnection(changed, account).route).toMatchObject({ imageProxyDns: false, confirmImageProxyDns: false })
    changed = changeDraft(changed, 'imageProxyDns', true)
    expect(draftToConnection(changed, account).route).toMatchObject({ imageProxyDns: true, confirmImageProxyDns: true })
    changed = changeDraft(changed, 'proxyPort', '1081')
    expect(draftToConnection(changed, account).route).toMatchObject({ imageProxyDns: false, confirmImageProxyDns: false })
  })
})
