import { describe, expect, it } from 'vitest'
import { changeDraft, draftToConnection, initialDraft } from './account-draft'
import { exampleAccount } from './test-fixtures'

describe('login username defaults', () => {
  it.each([
    ['', '', 'me@example.com', 'me@example.com'],
    [' \t ', '  ', 'me@example.com', 'me@example.com'],
    ['\uFEFF', '\u0085', 'me@example.com', 'me@example.com'],
    [' \t\u0085\uFEFF\u3000', '\uFEFF\u0085 ', 'me@example.com', 'me@example.com'],
    [' ReceiveUser ', '', 'ReceiveUser', 'me@example.com'],
    ['', ' SendUser ', 'me@example.com', 'SendUser'],
    [' ReceiveUser ', ' SendUser ', 'ReceiveUser', 'SendUser'],
    ['\uFEFFReceiveUser\uFEFF', '\u0085SendUser\u0085', 'ReceiveUser', 'SendUser'],
    [' \u0085\uFEFFReceiveUser\uFEFF\u0085 ', '\uFEFF\u0085 SendUser \u0085\uFEFF', 'ReceiveUser', 'SendUser'],
  ])('resolves IMAP %j and SMTP %j independently', (imapUsername, smtpUsername, expectedImap, expectedSmtp) => {
    const draft = initialDraft(null)
    draft.emailAddress = ' me@example.com '
    draft.imap = { ...draft.imap, host: 'imap.example.com', username: imapUsername }
    draft.smtp = { ...draft.smtp, host: 'smtp.example.com', username: smtpUsername }
    draft.sharedPassword.value = 'synthetic-secret'
    const connection = draftToConnection(draft, null)
    expect(connection).toMatchObject({ emailAddress: 'me@example.com', imap: { username: expectedImap }, smtp: { username: expectedSmtp } })
    expect(draft.imap.username).toBe(imapUsername)
    expect(draft.smtp.username).toBe(smtpUsername)
  })

  it('preserves saved usernames when the email changes and resolves only a cleared field', () => {
    const account = exampleAccount()
    const draft = initialDraft(account)
    draft.emailAddress = 'changed@example.com'
    expect(draftToConnection(draft, account)).toMatchObject({ imap: { username: account.imap.username }, smtp: { username: account.smtp.username } })
    draft.imap.username = ''
    expect(draftToConnection(draft, account)).toMatchObject({ imap: { username: 'changed@example.com' }, smtp: { username: account.smtp.username } })
  })

  it('retains keep actions when a cleared username resolves to the saved identity', () => {
    const account = exampleAccount()
    account.imap.username = account.emailAddress
    account.smtp.username = account.emailAddress
    const draft = initialDraft(account)
    draft.imap.username = ''
    draft.smtp.username = ''
    expect(draftToConnection(draft, account)).toMatchObject({
      imap: { username: account.emailAddress }, smtp: { username: account.emailAddress },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } },
    })
  })
})

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
