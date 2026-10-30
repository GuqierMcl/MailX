import { vi } from 'vitest'
import type { Account, AccountSyncState, FolderSyncState } from '@/generated/accounts'
import type { StartupStatus } from '@/generated/startup'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import type { MailxClient } from '@/lib/mailx-client'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'

export function exampleAccount(overrides: Partial<Account> = {}): Account {
  return {
    id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', revision: 3,
    displayName: '工作邮箱', color: null, senderName: null, useProviderIcon: true, emailAddress: 'work@example.com', providerId: null,
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
  let preferences: import('@/generated/accounts').AppSettings = { general: { language: 'zh-CN', autoCheckUpdates: true, closeBehavior: 'ask', showMenuBarIcon: false }, appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask', senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, compose: { defaultAccountId: '', undoSendSeconds: 0 } }
  let settingsRevision = 0
  const settingsSnapshot = () => ({ settings: preferences, effectiveLocale: preferences.general.language === 'en' ? 'en' as const : 'zh-CN' as const, revision: settingsRevision, stored: true, recovered: false })
  return {
    available: true,
    getStartupStatus: vi.fn<MailxClient['getStartupStatus']>().mockResolvedValue(startup),
    retryStartup: vi.fn<MailxClient['retryStartup']>().mockResolvedValue(startup),
    listAccounts: vi.fn<MailxClient['listAccounts']>().mockResolvedValue(rows),
    getAccount: vi.fn<MailxClient['getAccount']>().mockImplementation(async (id) => rows.find((row) => row.id === id) ?? exampleAccount()),
    createAccount: vi.fn<MailxClient['createAccount']>().mockImplementation(async ({ config }) => exampleAccount({ useProviderIcon: config.useProviderIcon ?? true })),
    updateConnection: vi.fn<MailxClient['updateConnection']>().mockImplementation(async ({ accountId, expectedRevision }) => ({ ...(rows.find((row) => row.id === accountId) ?? exampleAccount()), revision: expectedRevision + 1 })),
    updateProfile: vi.fn<MailxClient['updateProfile']>().mockImplementation(async ({ accountId, profile }) => ({ ...(rows.find((row) => row.id === accountId) ?? exampleAccount()), ...profile, useProviderIcon: profile.useProviderIcon ?? true })),
    removeAccount: vi.fn<MailxClient['removeAccount']>().mockImplementation(async ({ accountId }) => ({ accountId, draftIds: [], credentialsPending: false })),
    onAccountRemoved: vi.fn<MailxClient['onAccountRemoved']>().mockReturnValue(() => undefined),
    verifyAccountConnection: vi.fn<MailxClient['verifyAccountConnection']>().mockImplementation(async (accountId, revision) => ({
      accountId, revision,
      imap: { status: 'verified', stage: 'complete', code: null },
      smtp: { status: 'verified', stage: 'complete', code: null },
    })),
    verifyDraftConnection: vi.fn<MailxClient['verifyDraftConnection']>().mockResolvedValue({
      verificationId: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
      imap: { status: 'verified', stage: 'complete', code: null },
      smtp: { status: 'verified', stage: 'complete', code: null },
    }),
    openProviderHelp: vi.fn<MailxClient['openProviderHelp']>().mockResolvedValue(undefined),
    listFolders: vi.fn<MailxClient['listFolders']>().mockResolvedValue([]),
    listSyncStates: vi.fn<MailxClient['listSyncStates']>().mockResolvedValue(rows.map((account) => exampleSyncState({ accountId: account.id, mailboxSourceId: account.mailboxSourceId }))),
    setFlag: vi.fn<MailxClient['setFlag']>().mockResolvedValue('queued'),
    onFlagWriteBack: vi.fn<MailxClient['onFlagWriteBack']>().mockReturnValue(() => undefined),
    refreshSync: vi.fn<MailxClient['refreshSync']>().mockResolvedValue(undefined),
    getAvatar: vi.fn<MailxClient['getAvatar']>().mockResolvedValue(null),
    saveAttachment: vi.fn<MailxClient['saveAttachment']>().mockResolvedValue('saved'),
    clearImageCache: vi.fn<MailxClient['clearImageCache']>().mockResolvedValue(undefined),
    onSyncState: vi.fn<MailxClient['onSyncState']>().mockReturnValue(() => undefined),
    listEmails: vi.fn<MailxClient['listEmails']>().mockResolvedValue({ items: [], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, syncedAfter: null, syncedBefore: null, windowCapped: false } }),
    getEmail: vi.fn<MailxClient['getEmail']>().mockRejectedValue(new Error('not found')),
    fetchBody: vi.fn<MailxClient['fetchBody']>().mockRejectedValue(new Error('unavailable')),
    requestPreviews: vi.fn<MailxClient['requestPreviews']>().mockResolvedValue(undefined),
    onEmailPreviews: vi.fn<MailxClient['onEmailPreviews']>().mockReturnValue(() => undefined),
    prepareBody: vi.fn<MailxClient['prepareBody']>().mockResolvedValue({ documentToken: 'test-document' }),
    releaseBody: vi.fn<MailxClient['releaseBody']>().mockResolvedValue(undefined),
    imageStatus: vi.fn<MailxClient['imageStatus']>().mockResolvedValue([]),
    retryImages: vi.fn<MailxClient['retryImages']>().mockResolvedValue({ documentToken: 'retry-document' }),
    bodyLinks: vi.fn<MailxClient['bodyLinks']>().mockImplementation(async (_id, documentToken) => ({ documentToken, links: [] })),
    requestLink: vi.fn<MailxClient['requestLink']>().mockResolvedValue(undefined),
    openLink: vi.fn<MailxClient['openLink']>().mockResolvedValue(undefined),
    onLinkRequest: vi.fn<MailxClient['onLinkRequest']>().mockReturnValue(() => undefined),
    getSettings: vi.fn<MailxClient['getSettings']>().mockImplementation(async () => settingsSnapshot()),
    updateSettings: vi.fn<MailxClient['updateSettings']>().mockImplementation(async (patch) => { preferences = { ...preferences, ...patch }; settingsRevision += 1; return settingsSnapshot() }),
    onSettingsChanged: vi.fn<MailxClient['onSettingsChanged']>().mockImplementation(async () => () => undefined),
    testNotification: vi.fn<MailxClient['testNotification']>().mockResolvedValue(undefined),
    previewSound: vi.fn<MailxClient['previewSound']>().mockResolvedValue(undefined),
    onNotificationOpen: vi.fn<MailxClient['onNotificationOpen']>().mockReturnValue(() => undefined),
    createDraft: vi.fn<MailxClient['createDraft']>().mockRejectedValue(new Error('unused')),
    createDraftFrom: vi.fn<MailxClient['createDraftFrom']>().mockRejectedValue(new Error('unused')),
    listDrafts: vi.fn<MailxClient['listDrafts']>().mockResolvedValue([]),
    getDraft: vi.fn<MailxClient['getDraft']>().mockRejectedValue(new Error('unused')),
    updateDraft: vi.fn<MailxClient['updateDraft']>().mockRejectedValue(new Error('unused')),
    deleteDraft: vi.fn<MailxClient['deleteDraft']>().mockResolvedValue(undefined),
    addAttachments: vi.fn<MailxClient['addAttachments']>().mockResolvedValue(null),
    addDroppedAttachments: vi.fn<MailxClient['addDroppedAttachments']>().mockResolvedValue(null),
    addInlineImages: vi.fn<MailxClient['addInlineImages']>().mockResolvedValue(null),
    addInlineImageBytes: vi.fn<MailxClient['addInlineImageBytes']>().mockRejectedValue(new Error('unused')),
    inlineImagePreview: vi.fn<MailxClient['inlineImagePreview']>().mockRejectedValue(new Error('unused')),
    removeAttachment: vi.fn<MailxClient['removeAttachment']>().mockRejectedValue(new Error('unused')),
    attachmentPolicy: vi.fn<MailxClient['attachmentPolicy']>().mockResolvedValue({ maxTotalBytes: 20 * 1024 * 1024, maxCount: 20, serverLimitMb: null }),
    onFilesDropped: vi.fn<MailxClient['onFilesDropped']>().mockReturnValue(() => undefined),
    queueSend: vi.fn<MailxClient['queueSend']>().mockRejectedValue(new Error('unused')),
    listOutbox: vi.fn<MailxClient['listOutbox']>().mockResolvedValue([]),
    getOutbox: vi.fn<MailxClient['getOutbox']>().mockRejectedValue(new Error('unused')),
    cancelOutbox: vi.fn<MailxClient['cancelOutbox']>().mockRejectedValue(new Error('unused')),
    retryOutboxNow: vi.fn<MailxClient['retryOutboxNow']>().mockRejectedValue(new Error('unused')),
    removeLocalCopy: vi.fn<MailxClient['removeLocalCopy']>().mockRejectedValue(new Error('unused')),
    resolveUnknown: vi.fn<MailxClient['resolveUnknown']>().mockRejectedValue(new Error('unused')),
    openComposer: vi.fn<MailxClient['openComposer']>().mockResolvedValue(undefined),
    closeComposer: vi.fn<MailxClient['closeComposer']>().mockResolvedValue(undefined),
    cancelComposerClose: vi.fn<MailxClient['cancelComposerClose']>().mockResolvedValue(undefined),
    onOutbox: vi.fn<MailxClient['onOutbox']>().mockReturnValue(() => undefined),
    onSentCopiesChanged: vi.fn<MailxClient['onSentCopiesChanged']>().mockReturnValue(() => undefined),
    onDrafts: vi.fn<MailxClient['onDrafts']>().mockReturnValue(() => undefined),
    onAppClosing: vi.fn<MailxClient['onAppClosing']>().mockReturnValue(() => undefined),
    onAppCloseCancelled: vi.fn<MailxClient['onAppCloseCancelled']>().mockReturnValue(() => undefined),
    onComposerCloseRequested: vi.fn<MailxClient['onComposerCloseRequested']>().mockReturnValue(() => undefined),
    getUpdateState: vi.fn<MailxClient['getUpdateState']>().mockResolvedValue({ canInstall: true, phase: 'idle' }),
    checkForUpdates: vi.fn<MailxClient['checkForUpdates']>().mockResolvedValue({ canInstall: true, phase: 'idle' }),
    downloadUpdate: vi.fn<MailxClient['downloadUpdate']>().mockResolvedValue({ canInstall: true, phase: 'idle' }),
    cancelUpdate: vi.fn<MailxClient['cancelUpdate']>().mockResolvedValue({ canInstall: true, phase: 'available', version: '0.2.0', current: '0.1.0', notes: null, date: null }),
    restartToUpdate: vi.fn<MailxClient['restartToUpdate']>().mockResolvedValue({ canInstall: true, phase: 'ready', version: '0.2.0', current: '0.1.0', notes: null, date: null }),
    openUpdateLink: vi.fn<MailxClient['openUpdateLink']>().mockResolvedValue(undefined),
    onUpdateState: vi.fn<MailxClient['onUpdateState']>().mockReturnValue(() => undefined),
  }
}
