import { describe, expect, it } from 'vitest'
import { changeDraft, draftToConnection, initialDraft } from './account-draft'
import { exampleAccount } from './test-fixtures'
import { NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, QQ_PROVIDER } from './providers'
import { MailxClientError } from '@/lib/mailx-client'

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

describe('QQ provider domain boundary', () => {
  it.each(['qq.com.evil', 'evilqq.com', 'sub.vip.qq.com', 'vip.qq.com.evil', 'evilvip.qq.com', 'sub.qq.com'])('rejects %s rather than treating a suffix or subdomain as a supported mailbox', (domain) => {
    const draft = initialDraft(null, QQ_PROVIDER)
    draft.emailAddress = `synthetic@${domain}`
    draft.sharedPassword.value = 'synthetic-secret'
    try {
      draftToConnection(draft, null, QQ_PROVIDER)
      expect.fail('Unsupported QQ domain was accepted')
    } catch (cause) {
      expect(cause).toBeInstanceOf(MailxClientError)
      expect((cause as MailxClientError).failure).toMatchObject({ code: 'validation', field: 'emailAddress' })
    }
  })
})

describe.each([
  [NETEASE_163_PROVIDER, '163.com', '126.com'],
  [NETEASE_126_PROVIDER, '126.com', '163.com'],
] as const)('NetEase provider domain boundary: %s', (provider, domain, otherDomain) => {
  it.each([otherDomain, `sub.${domain}`, `${domain}.evil`, `evil${domain}`, 'qq.com', 'vip.163.com', 'vip.126.com', 'yeah.net'])('rejects %s before submitting a credential', (unsupportedDomain) => {
    const draft = initialDraft(null, provider)
    draft.emailAddress = `synthetic@${unsupportedDomain}`
    draft.sharedPassword.value = 'synthetic-authorization-code'
    try {
      draftToConnection(draft, null, provider)
      expect.fail('Unsupported NetEase domain was accepted')
    } catch (cause) {
      expect(cause).toBeInstanceOf(MailxClientError)
      expect((cause as MailxClientError).failure).toMatchObject({ code: 'validation', field: 'emailAddress' })
    }
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
