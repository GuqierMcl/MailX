import { describe, expect, it } from 'vitest'
import { changeDraft, changeDraftServer, draftServer, draftToConnection, draftToRoute, initialDraft } from './account-draft'
import { exampleAccount } from './test-fixtures'
import { FEISHU_PROVIDER, GMAIL_PROVIDER, ICLOUD_PROVIDER, LARK_PROVIDER, MAIL_139_PROVIDER, NETEASE_126_PROVIDER, NETEASE_163_PROVIDER, NETEASE_YEAH_PROVIDER, QQ_PROVIDER, SINA_PROVIDER, type MailProvider } from './providers'
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

describe('new provider address and server boundaries', () => {
  const credential = '  synthetic-secret  '
  function presetDraft(provider: MailProvider, emailAddress: string) {
    const draft = initialDraft(null, provider)
    return { ...draft, emailAddress, sharedPassword: { replace: true, value: credential } }
  }

  it.each([
    { provider: SINA_PROVIDER, address: 'Mailbox@SINA.COM', imap: 'imap.sina.com', smtp: 'smtp.sina.com' },
    { provider: SINA_PROVIDER, address: 'Mailbox@SINA.CN', imap: 'imap.sina.cn', smtp: 'smtp.sina.cn' },
    { provider: FEISHU_PROVIDER, address: 'Team.Member+Ops@Mail.Example.COM', imap: 'imap.feishu.cn', smtp: 'smtp.feishu.cn' },
    { provider: LARK_PROVIDER, address: 'Team.Member+Ops@Mail.Example.COM', imap: 'imap.larksuite.com', smtp: 'smtp.larksuite.com' },
  ])('resolves $address for $provider.id without rewriting the login or credential', ({ provider, address, imap, smtp }) => {
    const draft = presetDraft(provider, ` ${address} `)
    expect(draftToConnection(draft, null, provider)).toMatchObject({
      emailAddress: address, providerId: provider.id,
      imap: { host: imap, port: 993, tls: 'implicit', username: address },
      smtp: { host: smtp, port: 465, tls: 'implicit', username: address },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: credential } },
    })
  })

  it.each(['vip.sina.com', 'vip.sina.cn', 'sina.com.example.com', 'evil-sina.cn', 'sina.cn.', 'example.com'])('rejects the out-of-scope Sina domain %s', (domain) => {
    expect(() => draftToConnection(presetDraft(SINA_PROVIDER, `me@${domain}`), null, SINA_PROVIDER)).toThrow(
      expect.objectContaining({ failure: expect.objectContaining({ code: 'validation', field: 'emailAddress' }) }),
    )
  })

  describe.each([FEISHU_PROVIDER, LARK_PROVIDER])('$id organization addresses', (provider) => {
    it.each(['me@-company.com', 'me@company-.com', 'me@company..com', 'me@company.com/path', 'me@company.com:443', 'me@example.test@company.com', 'me\u0085@company.com', `me@${'a'.repeat(64)}.com`])('rejects a malformed address: %s', (address) => {
      expect(() => draftToConnection(presetDraft(provider, address), null, provider)).toThrow(
        expect.objectContaining({ failure: expect.objectContaining({ code: 'validation', field: 'emailAddress' }) }),
      )
    })

    it.each(['Mailbox+deliveries@team.company.example', '成员+分组@team.company.example', 'Mailbox@team.company.example.'])('preserves the login %s and leaves mailbox eligibility to the provider', (address) => {
      expect(draftToConnection(presetDraft(provider, address), null, provider)).toMatchObject({
        emailAddress: address, imap: { username: address }, smtp: { username: address },
      })
    })
  })

  it('keeps explicit Sina edits per field even when an override equals the primary default', () => {
    let draft = presetDraft(SINA_PROVIDER, 'me@sina.cn')
    draft = changeDraftServer(draft, 'imap', 'host', 'imap.sina.com')
    draft = changeDraftServer(draft, 'smtp', 'port', '587')
    draft = changeDraftServer(draft, 'smtp', 'tls', 'startTls')
    draft = changeDraftServer(draft, 'smtp', 'username', 'SendUser')
    for (const [address, smtpHost] of [['me@SINA.COM', 'smtp.sina.com'], ['me@SINA.CN', 'smtp.sina.cn']]) {
      draft = changeDraft(draft, 'emailAddress', address)
      expect(draftServer(draft, 'smtp', SINA_PROVIDER)).toMatchObject({ host: smtpHost, port: '587', tls: 'startTls' })
      expect(draftToConnection(draft, null, SINA_PROVIDER)).toMatchObject({
        imap: { host: 'imap.sina.com', port: 993, tls: 'implicit', username: address },
        smtp: { host: smtpHost, port: 587, tls: 'startTls', username: 'SendUser' },
      })
    }
  })

  it.each([
    { provider: FEISHU_PROVIDER, domain: 'feishu.cn', other: 'larksuite.com' },
    { provider: LARK_PROVIDER, domain: 'larksuite.com', other: 'feishu.cn' },
  ])('pins new $provider.id accounts even when the draft contains the other region’s servers', ({ provider, domain, other }) => {
    const draft = presetDraft(provider, 'me@team.example.com')
    draft.imap = { ...draft.imap, host: `imap.${other}`, port: '143', tls: 'startTls' }
    draft.smtp = { ...draft.smtp, host: `smtp.${other}`, port: '587', tls: 'startTls' }
    const routed = { ...draft, routeKind: 'custom' as const, proxyHost: 'proxy.example.com', proxyPort: '8118' }
    expect(draftToConnection(routed, null, provider)).toMatchObject({
      providerId: provider.id, imap: { host: `imap.${domain}`, port: 993, tls: 'implicit' },
      smtp: { host: `smtp.${domain}`, port: 465, tls: 'implicit' },
      route: { kind: 'custom', host: 'proxy.example.com', port: 8118 },
    })
    expect(draftToConnection(routed, null)).toMatchObject({
      providerId: null, imap: { host: `imap.${other}`, port: 143, tls: 'startTls' }, smtp: { host: `smtp.${other}`, port: 587, tls: 'startTls' },
    })
  })

  it.each([
    { provider: SINA_PROVIDER, savedDomain: 'example.com' },
    { provider: FEISHU_PROVIDER, savedDomain: 'larksuite.com' },
    { provider: LARK_PROVIDER, savedDomain: 'feishu.cn' },
  ])('preserves saved $provider.id endpoints at $savedDomain and credentials when editing the address', ({ provider, savedDomain }) => {
    const account = exampleAccount({
      providerId: provider.id, emailAddress: 'member@team.example.com',
      imap: { host: `imap.${savedDomain}`, port: 1993, tls: 'implicit', username: 'saved-in' },
      smtp: { host: `smtp.${savedDomain}`, port: 587, tls: 'startTls', username: 'saved-out' },
    })
    const draft = changeDraft(initialDraft(account, provider), 'emailAddress', 'edited@elsewhere.example')
    expect(draftToConnection(draft, account, provider)).toMatchObject({
      providerId: provider.id, emailAddress: 'edited@elsewhere.example', imap: account.imap, smtp: account.smtp,
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
  { provider: NETEASE_163_PROVIDER, domain: '163.com' },
  { provider: NETEASE_126_PROVIDER, domain: '126.com' },
  { provider: NETEASE_YEAH_PROVIDER, domain: 'yeah.net' },
])('NetEase provider domain boundary: $domain', ({ provider, domain }) => {
  it.each([...['163.com', '126.com', 'yeah.net'].filter((other) => other !== domain), `sub.${domain}`, `${domain}.evil`, `evil${domain}`, 'qq.com', 'vip.163.com', 'vip.126.com'])('rejects %s before submitting a credential', (unsupportedDomain) => {
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

describe('139 provider identity and domain boundary', () => {
  it.each(['sub.139.com', '139.com.evil', 'evil139.com', '10086.cn', 'example.com'])('rejects %s before submitting a credential', (domain) => {
    const draft = initialDraft(null, MAIL_139_PROVIDER)
    draft.emailAddress = `synthetic@${domain}`
    draft.sharedPassword.value = 'synthetic-code'
    expect(() => draftToConnection(draft, null, MAIL_139_PROVIDER)).toThrow(MailxClientError)
  })

  it('defaults to the complete address and preserves a code without a fixed length rule', () => {
    const draft = initialDraft(null, MAIL_139_PROVIDER)
    draft.emailAddress = 'Synthetic@139.COM'
    draft.sharedPassword.value = 'test-code'
    expect(draftToConnection(draft, null, MAIL_139_PROVIDER)).toMatchObject({
      providerId: '139', emailAddress: 'Synthetic@139.COM',
      imap: { username: 'Synthetic@139.COM' }, smtp: { username: 'Synthetic@139.COM' },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'test-code' } },
    })
    draft.imap.username = 'synthetic-phone-login'
    draft.smtp.username = 'different-synthetic-login'
    expect(draftToConnection(draft, null, MAIL_139_PROVIDER)).toMatchObject({
      emailAddress: 'Synthetic@139.COM', imap: { username: 'synthetic-phone-login' }, smtp: { username: 'different-synthetic-login' },
    })
  })
})

describe('iCloud provider domain and credential boundaries', () => {
  it.each(['sub.icloud.com', 'icloud.com.evil', 'evilicloud.com', 'sub.me.com', 'me.com.evil', 'sub.mac.com', 'mac.com.evil', 'example.com'])('rejects %s before submitting a credential', (domain) => {
    const draft = initialDraft(null, ICLOUD_PROVIDER)
    draft.emailAddress = `synthetic@${domain}`
    draft.sharedPassword.value = 'abcd-efgh-ijkl-mnop'
    expect(() => draftToConnection(draft, null, ICLOUD_PROVIDER)).toThrow(MailxClientError)
  })

  it('keeps independently entered IMAP and SMTP identities and an unchanged shared app password', () => {
    const draft = initialDraft(null, ICLOUD_PROVIDER)
    draft.emailAddress = 'Synthetic@ME.COM'
    draft.imap.username = ' Synthetic '
    draft.smtp.username = ' synthetic@icloud.com '
    draft.sharedPassword.value = ' abcd-efgh-ijkl-mnop '
    expect(draftToConnection(draft, null, ICLOUD_PROVIDER)).toMatchObject({
      providerId: 'icloud', emailAddress: 'Synthetic@ME.COM',
      imap: { username: 'Synthetic' }, smtp: { username: 'synthetic@icloud.com' },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: ' abcd-efgh-ijkl-mnop ' } },
    })
  })
})

describe('Gmail provider domain and credential boundaries', () => {
  it.each(['sub.gmail.com', 'gmail.com.evil', 'evilgmail.com', 'google.com', 'googlemail.com.evil', 'example.com'])('rejects %s before submitting a credential', (domain) => {
    const draft = initialDraft(null, GMAIL_PROVIDER)
    draft.emailAddress = `synthetic@${domain}`
    draft.sharedPassword.value = 'abcdefghijklmnop'
    expect(() => draftToConnection(draft, null, GMAIL_PROVIDER)).toThrow(MailxClientError)
  })

  it.each(['Synthetic@GMAIL.com', 'synthetic@googlemail.com'])('fills full-address logins for %s and keeps the app password as entered', (emailAddress) => {
    const draft = initialDraft(null, GMAIL_PROVIDER)
    draft.emailAddress = emailAddress
    draft.sharedPassword.value = 'abcd efgh ijkl mnop'
    expect(draftToConnection(draft, null, GMAIL_PROVIDER)).toMatchObject({
      providerId: 'gmail', emailAddress,
      imap: { host: 'imap.gmail.com', port: 993, tls: 'implicit', username: emailAddress },
      smtp: { host: 'smtp.gmail.com', port: 465, tls: 'implicit', username: emailAddress },
      authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'abcd efgh ijkl mnop' } },
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

describe('sign-in route', () => {
  it('has a route but needs no address, servers or passwords', () => {
    const draft = initialDraft(null)
    expect(draftToRoute(draft)).toEqual({ kind: 'direct' })
    expect(draftToRoute({ ...draft, routeKind: 'inherit' })).toEqual({ kind: 'inherit' })
  })

  it('carries a custom proxy, with its login, into sign-in', () => {
    const draft = { ...initialDraft(null), routeKind: 'custom' as const, proxyKind: 'httpConnect' as const, proxyHost: ' proxy.example.com ', proxyPort: '8080' }
    expect(draftToRoute({ ...draft, proxyHost: 'proxy.example.com' })).toMatchObject({ kind: 'custom', proxyKind: 'httpConnect', host: 'proxy.example.com', port: 8080, dns: 'proxy', authentication: null })
    const withLogin = draftToRoute({ ...draft, proxyHost: 'proxy.example.com', proxyAuth: true, proxyUsername: ' lin ', proxyPassword: { replace: true, value: 'synthetic' } })
    expect(withLogin).toMatchObject({ authentication: { username: 'lin', password: { action: 'replace', value: 'synthetic' } } })
  })

  it('rejects a broken proxy instead of quietly signing in directly', () => {
    const draft = { ...initialDraft(null), routeKind: 'custom' as const, proxyHost: 'proxy.example.com', proxyPort: '8080' }
    for (const broken of [{ proxyHost: '' }, { proxyHost: 'http://proxy.example.com' }, { proxyPort: '0' }, { proxyPort: 'abc' }, { proxyAuth: true, proxyUsername: '' }, { proxyAuth: true, proxyUsername: 'lin', proxyPassword: { replace: true, value: '' } }]) {
      expect(() => draftToRoute({ ...draft, ...broken })).toThrow(MailxClientError)
    }
  })
})
