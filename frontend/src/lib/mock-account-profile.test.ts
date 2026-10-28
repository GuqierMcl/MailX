import { describe, expect, it } from 'vitest'
import type { AccountConfigInput } from '@/generated/accounts'
import { createMockClient } from './mock-client'

function config(useProviderIcon = true): AccountConfigInput {
  return {
    displayName: 'QQ 账号', color: '#2563eb', senderName: null,
    useProviderIcon,
    emailAddress: 'synthetic@qq.com', providerId: 'qq',
    imap: { host: 'imap.qq.com', port: 993, tls: 'implicit', username: 'synthetic@qq.com' },
    smtp: { host: 'smtp.qq.com', port: 465, tls: 'implicit', username: 'synthetic@qq.com' },
    authentication: { kind: 'password', mode: 'shared', password: { action: 'replace', value: 'synthetic-secret' } },
    route: { kind: 'direct' },
  }
}

describe('mock account provider icon preference', () => {
  it('defaults new and seeded accounts to using provider icons', async () => {
    const client = createMockClient()
    expect((await client.listAccounts()).every((account) => account.useProviderIcon)).toBe(true)
    const account = await client.createAccount({ requestId: crypto.randomUUID(), config: config() })
    expect(account.useProviderIcon).toBe(true)
    expect((await client.getAccount(account.id)).useProviderIcon).toBe(true)
    // Older wire requests omit this field; the current generated input requires it.
    const legacy = { ...config(), useProviderIcon: undefined } as unknown as AccountConfigInput
    expect((await client.createAccount({ requestId: crypto.randomUUID(), config: legacy })).useProviderIcon).toBe(true)
  })

  it('preserves an explicitly disabled preference when a connection is replaced', async () => {
    const client = createMockClient()
    const account = await client.createAccount({ requestId: crypto.randomUUID(), config: config(false) })
    expect(account.useProviderIcon).toBe(false)
    const { emailAddress, providerId, imap, smtp, authentication, route } = config()
    const saved = await client.updateConnection({
      accountId: account.id, expectedRevision: account.revision,
      connection: { emailAddress, providerId, imap, smtp: { ...smtp, port: 587, tls: 'startTls' }, authentication, route },
    })
    expect(saved.useProviderIcon).toBe(false)
    expect(saved.revision).toBe(account.revision + 1)
  })

  it('saves the preference per account without changing another account or the connection revision', async () => {
    const client = createMockClient()
    const account = await client.createAccount({ requestId: crypto.randomUUID(), config: config() })
    const other = await client.createAccount({ requestId: crypto.randomUUID(), config: config() })
    const profile = { displayName: account.displayName, color: account.color, senderName: account.senderName }
    const saved = await client.updateProfile({ accountId: account.id, profile: { ...profile, useProviderIcon: false } })
    expect(saved).toMatchObject({ useProviderIcon: false, revision: account.revision, color: account.color })
    expect((await client.getAccount(other.id)).useProviderIcon).toBe(true)
    expect((await client.listAccounts()).find((row) => row.id === account.id)?.useProviderIcon).toBe(false)
    expect((await client.updateProfile({ accountId: account.id, profile: { ...profile, useProviderIcon: true } })).useProviderIcon).toBe(true)
  })
})
