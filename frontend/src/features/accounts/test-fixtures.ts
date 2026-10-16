import { vi } from 'vitest'
import type { Account } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'

export function exampleAccount(overrides: Partial<Account> = {}): Account {
  return {
    id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', revision: 3,
    displayName: '工作邮箱', emailAddress: 'work@example.com', providerId: null,
    mailboxSourceId: 'source-a', imap: { host: 'imap.example.com', port: 993, tls: 'implicit', username: 'receive@example.com' },
    smtp: { host: 'smtp.example.com', port: 465, tls: 'implicit', username: 'send@example.com' },
    authentication: { mode: 'shared', imap: { id: 'auth-a', generation: 1, passwordConfigured: true, trust: 'trusted' }, smtp: { id: 'auth-a', generation: 1, passwordConfigured: true, trust: 'trusted' } },
    route: { kind: 'direct' }, routeRevision: 1, defaultRouteRevision: null, connectionStatus: 'unverified',
    createdAt: '2026-10-16T00:00:00Z', updatedAt: '2026-10-16T00:00:00Z', ...overrides,
  }
}

export function testClient(rows: Account[] = []) {
  return {
    available: true,
    listAccounts: vi.fn<MailxClient['listAccounts']>().mockResolvedValue(rows),
    getAccount: vi.fn<MailxClient['getAccount']>().mockImplementation(async (id) => rows.find((row) => row.id === id) ?? exampleAccount()),
    createAccount: vi.fn<MailxClient['createAccount']>().mockResolvedValue(exampleAccount()),
    updateAccount: vi.fn<MailxClient['updateAccount']>().mockResolvedValue(exampleAccount({ revision: 4 })),
  }
}
