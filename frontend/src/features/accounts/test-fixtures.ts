import { vi } from 'vitest'
import type { Account, AccountSyncState, FolderSyncState } from '@/generated/accounts'
import type { StartupStatus } from '@/generated/startup'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
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

export function exampleFolderSyncState(folderId: string, overrides: Partial<FolderSyncState> = {}): FolderSyncState {
  return { folderId, phase: 'idle', lastSyncedAt: '2026-10-18T08:00:00.000Z', failure: null, dataRevision: 0, ...overrides }
}

export function exampleSyncState(overrides: Partial<AccountSyncState> = {}): AccountSyncState {
  return {
    accountId: exampleAccount().id, mailboxSourceId: 'source-a', generation: 1, sequence: 1, folderRevision: 0,
    phase: 'idle', lastSyncedAt: '2026-10-18T08:00:00.000Z', failure: null, folders: [], ...overrides,
  }
}

export function testClient(rows: Account[] = [], startup: StartupStatus = { state: 'ready', revision: 1 }) {
  let preferences: import('@/generated/accounts').AppSettings = { appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask', senderAvatars: false } }
  return {
    available: true,
    getStartupStatus: vi.fn<MailxClient['getStartupStatus']>().mockResolvedValue(startup),
    retryStartup: vi.fn<MailxClient['retryStartup']>().mockResolvedValue(startup),
    listAccounts: vi.fn<MailxClient['listAccounts']>().mockResolvedValue(rows),
    getAccount: vi.fn<MailxClient['getAccount']>().mockImplementation(async (id) => rows.find((row) => row.id === id) ?? exampleAccount()),
    createAccount: vi.fn<MailxClient['createAccount']>().mockResolvedValue(exampleAccount()),
    updateAccount: vi.fn<MailxClient['updateAccount']>().mockResolvedValue(exampleAccount({ revision: 4 })),
    listFolders: vi.fn<MailxClient['listFolders']>().mockResolvedValue([]),
    listSyncStates: vi.fn<MailxClient['listSyncStates']>().mockResolvedValue(rows.map((account) => exampleSyncState({ accountId: account.id, mailboxSourceId: account.mailboxSourceId }))),
    refreshSync: vi.fn<MailxClient['refreshSync']>().mockResolvedValue(undefined),
    getAvatar: vi.fn<MailxClient['getAvatar']>().mockResolvedValue(null),
    onSyncState: vi.fn<MailxClient['onSyncState']>().mockReturnValue(() => undefined),
    listEmails: vi.fn<MailxClient['listEmails']>().mockResolvedValue({ items: [], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, syncedAfter: null, syncedBefore: null, windowCapped: false } }),
    getEmail: vi.fn<MailxClient['getEmail']>().mockRejectedValue(new Error('not found')),
    fetchBody: vi.fn<MailxClient['fetchBody']>().mockRejectedValue(new Error('unavailable')),
    prepareBody: vi.fn<MailxClient['prepareBody']>().mockResolvedValue({ documentToken: 'test-document' }),
    releaseBody: vi.fn<MailxClient['releaseBody']>().mockResolvedValue(undefined),
    bodyLinks: vi.fn<MailxClient['bodyLinks']>().mockImplementation(async (_id, documentToken) => ({ documentToken, links: [] })),
    requestLink: vi.fn<MailxClient['requestLink']>().mockResolvedValue(undefined),
    openLink: vi.fn<MailxClient['openLink']>().mockResolvedValue(undefined),
    onLinkRequest: vi.fn<MailxClient['onLinkRequest']>().mockReturnValue(() => undefined),
    getSettings: vi.fn<MailxClient['getSettings']>().mockResolvedValue({ settings: preferences, stored: true, recovered: false }),
    updateSettings: vi.fn<MailxClient['updateSettings']>().mockImplementation(async (patch) => { preferences = { ...preferences, ...patch }; return { settings: preferences, stored: true, recovered: false } }),
  }
}
