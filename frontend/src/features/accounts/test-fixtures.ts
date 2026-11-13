import { vi } from 'vitest'
import type { Account, AccountSyncState, FolderSyncState } from '@/generated/accounts'
import type { StartupStatus } from '@/generated/startup'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import type { MailxClient } from '@/lib/mailx-client'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'

export function exampleAccount(): Account & { authentication: Extract<Account['authentication'], { kind: 'password' }> }
export function exampleAccount(overrides: Partial<Account>): Account
export function exampleAccount(overrides: Partial<Account> = {}): Account {
  return {
    id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', revision: 3,
    displayName: '工作邮箱', color: null, senderName: null, useProviderIcon: true, emailAddress: 'work@example.com', providerId: null,
    mailboxSourceId: 'source-a', imap: { host: 'imap.example.com', port: 993, tls: 'implicit', username: 'receive@example.com' },
    smtp: { host: 'smtp.example.com', port: 465, tls: 'implicit', username: 'send@example.com' },
    authentication: { kind: 'password', mode: 'shared', imap: { id: 'auth-a', generation: 1, passwordConfigured: true, trust: 'trusted' }, smtp: { id: 'auth-a', generation: 1, passwordConfigured: true, trust: 'trusted' } },
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
  let preferences: import('@/generated/accounts').AppSettings = { general: { language: 'zh-CN', autoUpdate: true, closeBehavior: 'ask', showMenuBarIcon: true, silentStart: false }, appearance: DEFAULT_APPEARANCE, security: { remoteImages: 'ask', senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, compose: { defaultAccountId: '', undoSendSeconds: 0 }, mcp: { enabled: false }, translation: { provider: null, microsoftRegion: '', route: { kind: 'direct' }, targetLanguage: 'auto', suggest: true, neverSuggest: [], alwaysTranslate: [], includeQuotes: false, consentedProvider: null, consentedAi: null } }
  let settingsRevision = 0
  let autostart = { enabled: false, available: true }
  const settingsSnapshot = () => ({ settings: preferences, effectiveLocale: preferences.general.language === 'en' ? 'en' as const : 'zh-CN' as const, revision: settingsRevision, stored: true, recovered: false })
  return {
    available: true,
    getAutostart: vi.fn<MailxClient['getAutostart']>().mockImplementation(async () => ({ ...autostart })),
    setAutostart: vi.fn<MailxClient['setAutostart']>().mockImplementation(async (enabled) => (autostart = { enabled, available: true })),
    getStartupStatus: vi.fn<MailxClient['getStartupStatus']>().mockResolvedValue(startup),
    retryStartup: vi.fn<MailxClient['retryStartup']>().mockResolvedValue(startup),
    listAccounts: vi.fn<MailxClient['listAccounts']>().mockResolvedValue(rows),
    getAccount: vi.fn<MailxClient['getAccount']>().mockImplementation(async (id) => rows.find((row) => row.id === id) ?? exampleAccount()),
    createAccount: vi.fn<MailxClient['createAccount']>().mockImplementation(async ({ config }) => exampleAccount({ useProviderIcon: config.useProviderIcon ?? true })),
    beginOAuthAccount: vi.fn<MailxClient['beginOAuthAccount']>().mockResolvedValue({ sessionId: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd' }),
    finishOAuthAccount: vi.fn<MailxClient['finishOAuthAccount']>().mockResolvedValue({ kind: 'verified', emailAddress: 'lin@outlook.com' }),
    confirmOAuthMailbox: vi.fn<MailxClient['confirmOAuthMailbox']>().mockImplementation(async ({ emailAddress }) => ({ kind: 'verified', emailAddress })),
    createOAuthAccount: vi.fn<MailxClient['createOAuthAccount']>().mockImplementation(async ({ profile }) => exampleAccount({ useProviderIcon: profile.useProviderIcon })),
    finishOAuthReauthorization: vi.fn<MailxClient['finishOAuthReauthorization']>().mockImplementation(async () => exampleAccount()),
    cancelOAuthAccount: vi.fn<MailxClient['cancelOAuthAccount']>().mockResolvedValue(undefined),
    reauthorizeOAuthAccount: vi.fn<MailxClient['reauthorizeOAuthAccount']>().mockResolvedValue({ sessionId: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd' }),
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
    deleteEmails: vi.fn<MailxClient['deleteEmails']>().mockResolvedValue({ batchId: 'batch-1', undoUntil: '2099-01-01T00:00:00.000Z', permanent: false, skippedIds: [] }),
    undoDelete: vi.fn<MailxClient['undoDelete']>().mockResolvedValue(undefined),
    archiveEmails: vi.fn<MailxClient['archiveEmails']>().mockResolvedValue({ processedCount: 1, changedCount: 1, undoToken: 'archive-1', skippedIds: [] }),
    unarchiveEmails: vi.fn<MailxClient['unarchiveEmails']>().mockResolvedValue({ processedCount: 1, changedCount: 1, undoToken: 'unarchive-1', skippedIds: [] }),
    setFlags: vi.fn<MailxClient['setFlags']>().mockImplementation(async (emailIds) => ({ results: emailIds.map((emailId) => ({ emailId, outcome: 'queued' as const, code: null })) })),
    selectEmails: vi.fn<MailxClient['selectEmails']>().mockResolvedValue({ items: [], truncated: false }),
    listContactAggregations: vi.fn<MailxClient['listContactAggregations']>().mockResolvedValue([]),
    setContactAggregation: vi.fn<MailxClient['setContactAggregation']>().mockResolvedValue(undefined),
    undoArchive: vi.fn<MailxClient['undoArchive']>().mockResolvedValue(undefined),
    onMailStateChanged: vi.fn<MailxClient['onMailStateChanged']>().mockReturnValue(() => undefined),
    onDeletions: vi.fn<MailxClient['onDeletions']>().mockReturnValue(() => undefined),
    refreshSync: vi.fn<MailxClient['refreshSync']>().mockResolvedValue(undefined),
    getAvatar: vi.fn<MailxClient['getAvatar']>().mockResolvedValue(null),
    saveAttachment: vi.fn<MailxClient['saveAttachment']>().mockResolvedValue('saved'),
    clearImageCache: vi.fn<MailxClient['clearImageCache']>().mockResolvedValue(undefined),
    translationStatus: vi.fn<MailxClient['translationStatus']>().mockImplementation(async (emailId) => ({ emailId, targetLanguage: 'en', detectedLanguage: null, detectionReliable: false, suggested: false, autoTranslate: false, translation: null })),
    translateEmail: vi.fn<MailxClient['translateEmail']>().mockRejectedValue(new Error('not configured')),
    cancelTranslation: vi.fn<MailxClient['cancelTranslation']>().mockResolvedValue(undefined),
    clearTranslation: vi.fn<MailxClient['clearTranslation']>().mockResolvedValue(undefined),
    clearTranslations: vi.fn<MailxClient['clearTranslations']>().mockResolvedValue(undefined),
    translationCacheStats: vi.fn<MailxClient['translationCacheStats']>().mockResolvedValue({ count: 0, bytes: 0 }),
    translationKeys: vi.fn<MailxClient['translationKeys']>().mockResolvedValue([]),
    setTranslationKey: vi.fn<MailxClient['setTranslationKey']>().mockResolvedValue([]),
    removeTranslationKey: vi.fn<MailxClient['removeTranslationKey']>().mockResolvedValue([]),
    translationProxyPassword: vi.fn<MailxClient['translationProxyPassword']>().mockResolvedValue({ configured: false }),
    setTranslationProxyPassword: vi.fn<MailxClient['setTranslationProxyPassword']>().mockResolvedValue({ configured: true }),
    removeTranslationProxyPassword: vi.fn<MailxClient['removeTranslationProxyPassword']>().mockResolvedValue({ configured: false }),
    testTranslation: vi.fn<MailxClient['testTranslation']>().mockRejectedValue(new Error('not configured')),
    aiState: vi.fn<MailxClient['aiState']>().mockResolvedValue({ connections: [], defaultModel: null, translationModel: null, route: { kind: 'direct' }, proxyPasswordSet: false }),
    aiSaveConnection: vi.fn<MailxClient['aiSaveConnection']>().mockRejectedValue(new Error('not configured')),
    aiRemoveConnection: vi.fn<MailxClient['aiRemoveConnection']>().mockRejectedValue(new Error('not configured')),
    aiSetDefaultModel: vi.fn<MailxClient['aiSetDefaultModel']>().mockRejectedValue(new Error('not configured')),
    aiSetTranslationModel: vi.fn<MailxClient['aiSetTranslationModel']>().mockRejectedValue(new Error('not configured')),
    aiSetRoute: vi.fn<MailxClient['aiSetRoute']>().mockRejectedValue(new Error('not configured')),
    aiSetProxyPassword: vi.fn<MailxClient['aiSetProxyPassword']>().mockRejectedValue(new Error('not configured')),
    aiRemoveProxyPassword: vi.fn<MailxClient['aiRemoveProxyPassword']>().mockRejectedValue(new Error('not configured')),
    aiCatalog: vi.fn<MailxClient['aiCatalog']>().mockResolvedValue({ providers: [], updatedAt: null, stale: false, refreshError: null }),
    aiListModels: vi.fn<MailxClient['aiListModels']>().mockResolvedValue([]),
    aiTest: vi.fn<MailxClient['aiTest']>().mockRejectedValue(new Error('not configured')),
    aiProviderIcon: vi.fn<MailxClient['aiProviderIcon']>().mockResolvedValue(null),
    onSyncState: vi.fn<MailxClient['onSyncState']>().mockReturnValue(() => undefined),
    listEmails: vi.fn<MailxClient['listEmails']>().mockResolvedValue({ items: [], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }),
    getEmail: vi.fn<MailxClient['getEmail']>().mockRejectedValue(new Error('not found')),
    getConversation: vi.fn<MailxClient['getConversation']>().mockRejectedValue(new Error('not found')),
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
    updateSettings: vi.fn<MailxClient['updateSettings']>().mockImplementation(async (patch) => { preferences = { ...preferences, ...patch, layout: { ...preferences.layout, ...patch.layout } }; settingsRevision += 1; return settingsSnapshot() }),
    onSettingsChanged: vi.fn<MailxClient['onSettingsChanged']>().mockImplementation(async () => () => undefined),
    testNotification: vi.fn<MailxClient['testNotification']>().mockResolvedValue(undefined),
    previewSound: vi.fn<MailxClient['previewSound']>().mockResolvedValue(undefined),
    onNotificationOpen: vi.fn<MailxClient['onNotificationOpen']>().mockReturnValue(() => undefined),
    createDraft: vi.fn<MailxClient['createDraft']>().mockRejectedValue(new Error('unused')),
    createDraftFrom: vi.fn<MailxClient['createDraftFrom']>().mockRejectedValue(new Error('unused')),
    listDrafts: vi.fn<MailxClient['listDrafts']>().mockResolvedValue([]),
    getDraft: vi.fn<MailxClient['getDraft']>().mockRejectedValue(new Error('unused')),
    prepareDraftBody: vi.fn<MailxClient['prepareDraftBody']>().mockResolvedValue({ documentToken: 'test-draft-document' }),
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
    openExternalLink: vi.fn<MailxClient['openExternalLink']>().mockResolvedValue(undefined),
    getMcpStatus: vi.fn<MailxClient['getMcpStatus']>().mockResolvedValue({ state: 'off' }),
    setMcpEnabled: vi.fn<MailxClient['setMcpEnabled']>().mockImplementation(async (enabled) => {
      preferences = { ...preferences, mcp: { enabled } }
      settingsRevision += 1
      return enabled ? { state: 'running', clients: [] } : { state: 'off' }
    }),
    getMcpClientConfig: vi.fn<MailxClient['getMcpClientConfig']>().mockResolvedValue({ command: '/opt/MailX/mailx-mcp', args: ['--discovery', '/data/mcp-discovery.json'], display: { command: '/opt/MailX/mailx-mcp', discovery: '~/data/mcp-discovery.json' } }),
    onMcpStatus: vi.fn<MailxClient['onMcpStatus']>().mockReturnValue(() => undefined),
    onUpdateState: vi.fn<MailxClient['onUpdateState']>().mockReturnValue(() => undefined),
  }
}
