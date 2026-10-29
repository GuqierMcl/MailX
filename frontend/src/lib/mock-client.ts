import type {
  Account,
  AccountRemoval,
  AccountConfigInput,
  Draft,
  DraftChanged,
  DraftSummary,
  OutboxChanged,
  OutboxItem,
  OutboxState,
  EmailDetail,
  EmailSummary,
  Folder,
  FolderRole,
  MailAddress,
  RouteConfig,
} from '@/generated/accounts'
import { cachedAppearance, parseFontFamilies } from '@/lib/appearance'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'
import { SHOWCASE_DATASET } from '@/lib/mock-showcase'
import { mockDraftBody, mockPlainToHtml } from './mock-draft-body'
import { cachedLanguage, systemLocale } from '@/i18n/locale'

/**
 * In-memory client for designing the UI in a plain browser (`bun run dev`).
 * Data is synthetic and lost on reload; nothing touches the network or disk.
 * Settings start from the first-paint appearance cache so a reload keeps them.
 */
export type MockClientOptions = {
  /** Time from the end of the undo window to `sending`, and from `sending` to `accepted`. Default 600 ms each. */
  sendStepMs?: number
  /** Browser preview only: called when the main window "opens" a composer, e.g. to open a tab for it. */
  onOpenComposer?: (draft: Draft, fresh: boolean) => void
  /**
   * Browser preview only: a composer tab starts with an empty mock, so it asks here for the draft
   * its URL names. Return `null` and the mock makes a blank draft with that id.
   */
  resolveDraft?: (id: string) => Draft | null
  /** Browser preview only: `showcase` loads the fictitious screenshot dataset. */
  seed?: string | null
}

export function createMockClient(options: MockClientOptions = {}): MailxClient {
  const dataset = options.seed === 'showcase' ? showcaseDataset() : DEFAULT_DATASET
  const accounts = dataset.accounts.map((account) => structuredClone(account))
  const emails = buildEmails(dataset)
  const folders = accounts.flatMap((account) => buildFolders(account.id, emails, dataset.customFolders))
  let settings: import('@/generated/accounts').SettingsSnapshot = { effectiveLocale: cachedLanguage().effectiveLocale, revision: 1, settings: { general: { language: cachedLanguage().language, autoCheckUpdates: true, closeBehavior: 'ask', showMenuBarIcon: false }, appearance: cachedAppearance(), security: { remoteImages: 'ask' as const, senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, compose: { defaultAccountId: '', undoSendSeconds: 0 } }, stored: true, recovered: false }

  function readStoredSettings() {
    try {
      const value = JSON.parse(localStorage.getItem('mailx.mock.settings') ?? 'null') as typeof settings | null
      if (value && Number.isSafeInteger(value.revision) && value.revision >= settings.revision &&
          ['system', 'zh-CN', 'en'].includes(value.settings?.general?.language) &&
          ['en', 'zh-CN'].includes(value.effectiveLocale) &&
          ['appearance', 'security', 'layout', 'notifications', 'compose'].every((key) =>
            typeof value.settings[key as keyof typeof value.settings] === 'object')) settings = value
    } catch { /* The browser preview cache is optional. */ }
  }
  readStoredSettings()

  const notFound = () => new MailxClientError(missing)
  const sendStepMs = options.sendStepMs ?? 600
  const outbox = createMockOutbox(sendStepMs)
  const drafts = new Map<string, Draft>()
  const draftsByMutation = new Map<string, string>()
  const draftListeners = new Set<(change: DraftChanged) => void>()
  const lockedDrafts = new Set<string>()
  const removedDrafts = new Set<string>()
  const removals = new Map<string, { revision: number; result: AccountRemoval }>()
  const removalListeners = new Set<(removal: AccountRemoval) => void>()
  const announceDraft = (change: DraftChanged) => draftListeners.forEach((listener) => listener(structuredClone(change)))
  const clone = <T,>(value: T) => structuredClone(value)

  const MOCK_ATTACHMENT_LIMIT = 20 * 1024 * 1024
  const MOCK_ATTACHMENT_SAMPLES: [string, string, number][] = [
    ['季度报表.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', 86_000],
    ['产品演示.pdf', 'application/pdf', 1_840_000],
    ['现场照片.png', 'image/png', 8_500_000],
    ['完整录屏.mp4', 'video/mp4', 12_000_000],
  ]
  let mockAttachmentCursor = 0

  function mutableDraft(id: string, expectedRevision: number) {
    const draft = requireDraft(id)
    if (lockedDrafts.has(id)) throw new MailxClientError({ code: 'draftLocked', message: '草稿已进入发送队列，不能修改。', field: null })
    if (draft.revision !== expectedRevision) throw new MailxClientError({ code: 'conflict', message: '草稿已被修改，请重新加载后重试。', field: null })
    return draft
  }

  function touchDraft(draft: Draft) {
    draft.revision += 1
    draft.updatedAt = new Date().toISOString()
    announceDraft({ draftId: draft.id, accountId: draft.accountId, change: 'updated' })
    return clone(draft)
  }

  function addMockAttachment(draft: Draft, [fileName, mediaType, size]: [string, string, number]) {
    const total = draft.attachments.reduce((sum, a) => sum + a.size, 0)
    if (draft.attachments.length >= 20) throw new MailxClientError({ code: 'tooManyAttachments', message: '附件最多 20 个。', field: null })
    if (total + size > MOCK_ATTACHMENT_LIMIT) throw new MailxClientError({ code: 'attachmentTooLarge', message: '附件总大小超过 20 MB。', field: null })
    draft.attachments.push({ id: crypto.randomUUID(), fileName, mediaType, size, origin: 'user' })
    return touchDraft(draft)
  }

  function requireDraft(id: string) {
    if (removedDrafts.has(id)) throw notFound()
    const draft = drafts.get(id) ?? adoptDraft(id)
    if (!draft) throw notFound()
    return draft
  }
  // Browser preview of `?composer=<id>`: the page's own mock has no such draft yet.
  function adoptDraft(id: string) {
    if (!options.resolveDraft) return null
    const now = new Date().toISOString()
    const found = options.resolveDraft(id)
    const draft: Draft = found ?? {
      id, accountId: accounts[0]?.id ?? '', kind: 'new', sourceEmailId: null, to: [], cc: [], bcc: [], subject: '', bodyHtml: '<p></p>', bodyText: '',
      inReplyTo: null, references: null, createdAt: now, updatedAt: now, revision: 1, attachments: [],
    }
    if (!accounts.some((row) => row.id === draft.accountId)) draft.accountId = accounts[0]?.id ?? ''
    drafts.set(id, draft)
    return draft
  }
  function newDraft(draft: Omit<Draft, 'id' | 'createdAt' | 'updatedAt' | 'revision' | 'attachments'>) {
    const now = new Date().toISOString()
    const created: Draft = { ...draft, id: crypto.randomUUID(), createdAt: now, updatedAt: now, revision: 1, attachments: [] }
    drafts.set(created.id, created)
    announceDraft({ draftId: created.id, accountId: created.accountId, change: 'created' })
    return created
  }
  const summary = (draft: Draft): DraftSummary => ({
    id: draft.id, accountId: draft.accountId, kind: draft.kind, sourceEmailId: draft.sourceEmailId, to: draft.to,
    subject: draft.subject, preview: draft.bodyText.replace(/\s+/g, ' ').slice(0, 120), updatedAt: draft.updatedAt, revision: draft.revision,
  })

  // A simulated send: never contacts anything. The item moves queued -> sending -> accepted on timers.
  outbox.onAccepted = (item) => {
    if (item.draftId && drafts.delete(item.draftId)) {
      lockedDrafts.delete(item.draftId)
      announceDraft({ draftId: item.draftId, accountId: item.accountId, change: 'deleted' })
    }
  }
  const unlock = (item: OutboxItem) => {
    if (item.draftId && lockedDrafts.delete(item.draftId)) {
      announceDraft({ draftId: item.draftId, accountId: item.accountId, change: 'unlocked' })
    }
  }
  outbox.onCanceled = unlock
  // A failed send leaves the draft editable again; `unknown` keeps it locked.
  outbox.onFailed = unlock

  return {
    available: true,
    getStartupStatus: async () => ({ state: 'ready', revision: 1 }),
    retryStartup: async () => ({ state: 'ready', revision: 1 }),
    listAccounts: async () => structuredClone(accounts),
    async getAccount(accountId) {
      const account = accounts.find((row) => row.id === accountId)
      if (!account) throw notFound()
      return structuredClone(account)
    },
    async verifyAccountConnection(accountId, expectedRevision, signal) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
      const account = accounts.find((row) => row.id === accountId)
      if (!account) throw notFound()
      if (account.revision !== expectedRevision) throw new MailxClientError({ code: 'conflict', message: '配置已被修改，请重新加载后重试。', field: null })
      const check = (configured: boolean, trust: string): import('@/generated/accounts').ProtocolVerification => !configured
        ? { status: 'failed', stage: 'configuration', code: 'credentialMissing' }
        : trust !== 'trusted' ? { status: 'failed', stage: 'configuration', code: 'credentialUntrusted' }
        : { status: 'verified', stage: 'complete', code: null }
      return { accountId, revision: account.revision, imap: check(account.authentication.imap.passwordConfigured, account.authentication.imap.trust), smtp: check(account.authentication.smtp.passwordConfigured, account.authentication.smtp.trust) }
    },
    async verifyDraftConnection(connection, signal) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
      const authentication = connection.authentication
      if (authentication.kind !== 'password') throw new MailxClientError({ code: 'unsupported', message: '', field: 'authentication' })
      const secrets: [import('@/generated/accounts').SecretUpdate, string][] = authentication.mode === 'shared'
        ? [[authentication.password, 'sharedPassword']]
        : [[authentication.imapPassword, 'imapPassword'], [authentication.smtpPassword, 'smtpPassword']]
      if (connection.route.kind === 'custom' && connection.route.authentication) secrets.push([connection.route.authentication.password, 'proxyPassword'])
      for (const [secret, field] of secrets) {
        if (secret.action !== 'replace' || !secret.value) throw new MailxClientError({ code: 'validation', message: '', field })
      }
      const verified = { status: 'verified' as const, stage: 'complete' as const, code: null }
      return { verificationId: crypto.randomUUID(), imap: { ...verified }, smtp: { ...verified } }
    },
    async openProviderHelp(providerId) {
      const url = providerId === 'qq' ? 'https://help.mail.qq.com/detail/0/1087'
        : providerId === '139' ? 'https://help.mail.10086.cn/statichtml/22/Content/1234606.html'
          : providerId === 'netease163' || providerId === 'netease126' || providerId === 'neteaseYeah'
            ? 'https://help.mail.163.com/faqDetail.do?code=d7a5dc8471cd0c0e8b4b8f4f8e49998b374173cfe9171305fa1ce630d7f67ac2a5feb28b66796d3b'
            : null
      if (!url) throw new MailxClientError({ code: 'validation', message: '配置参数无效。', field: 'providerId' })
      window.open(url, '_blank', 'noopener,noreferrer')
    },
    async createAccount({ config }) {
      const account = accountFromConfig(crypto.randomUUID(), 1, config)
      accounts.push(account)
      folders.push(...buildFolders(account.id, emails, dataset.customFolders))
      return structuredClone(account)
    },
    async updateConnection({ accountId, expectedRevision, connection }) {
      const index = accounts.findIndex((row) => row.id === accountId)
      if (index < 0) throw notFound()
      const current = accounts[index]
      if (current.revision !== expectedRevision) {
        throw new MailxClientError({ code: 'conflict', message: '配置已被修改，请重新加载后重试。', field: null })
      }
      const config = { ...connection, displayName: current.displayName, color: current.color, senderName: current.senderName, useProviderIcon: current.useProviderIcon }
      accounts[index] = { ...accountFromConfig(accountId, expectedRevision + 1, config), createdAt: current.createdAt }
      return structuredClone(accounts[index])
    },
    async updateProfile({ accountId, profile }) {
      const index = accounts.findIndex((row) => row.id === accountId)
      if (index < 0) throw notFound()
      const displayName = profile.displayName.trim()
      if (!displayName) throw new MailxClientError({ code: 'validation', message: '请填写账号名称。', field: 'displayName' })
      accounts[index] = { ...accounts[index], displayName, color: profile.color?.toLowerCase() || null, senderName: profile.senderName?.trim() || null, useProviderIcon: profile.useProviderIcon ?? true, updatedAt: new Date().toISOString() }
      return structuredClone(accounts[index])
    },
    async removeAccount({ accountId, expectedRevision }) {
      const receipt = removals.get(accountId)
      if (receipt) {
        if (receipt.revision !== expectedRevision) throw new MailxClientError({ code: 'conflict', message: '', field: null })
        removalListeners.forEach((listener) => listener(clone(receipt.result)))
        return clone(receipt.result)
      }
      const index = accounts.findIndex((account) => account.id === accountId)
      if (index < 0) throw notFound()
      if (accounts[index].revision !== expectedRevision) throw new MailxClientError({ code: 'conflict', message: '', field: null })
      // Check before changing any account data, just like the core transaction.
      outbox.removeAccount(accountId)
      const draftIds = [...drafts.values()].filter((draft) => draft.accountId === accountId).map((draft) => draft.id)
      accounts.splice(index, 1)
      for (let i = folders.length - 1; i >= 0; i--) if (folders[i].accountId === accountId) folders.splice(i, 1)
      for (let i = emails.length - 1; i >= 0; i--) if (emails[i].summary.accountId === accountId) emails.splice(i, 1)
      for (const draftId of draftIds) {
        drafts.delete(draftId)
        removedDrafts.add(draftId)
        lockedDrafts.delete(draftId)
      }
      const result = { accountId, draftIds, credentialsPending: false }
      removals.set(accountId, { revision: expectedRevision, result })
      removalListeners.forEach((listener) => listener(clone(result)))
      for (const draftId of draftIds) announceDraft({ accountId, draftId, change: 'deleted' })
      return clone(result)
    },
    onAccountRemoved(listener) { removalListeners.add(listener); return () => { removalListeners.delete(listener) } },
    listFolders: async (accountId) => structuredClone(folders.filter((folder) => folder.accountId === accountId)),
    listSyncStates: async () =>
      accounts.map((account) => ({
        accountId: account.id, mailboxSourceId: account.mailboxSourceId, generation: 1, sequence: 1, folderRevision: 1,
        phase: 'idle', lastSyncedAt: new Date().toISOString(), failure: null,
        folders: folders.filter((folder) => folder.accountId === account.id && folder.selectable).map((folder) => ({
          folderId: folder.id, phase: 'idle', lastSyncedAt: new Date().toISOString(), failure: null, dataRevision: 0,
        })),
      })),
    onSyncState: () => () => undefined,
    async listEmails(query) {
      const text = query.text?.trim().toLowerCase()
      const items = emails
        .map((email) => email.summary)
        .filter((email) =>
          (!query.accountIds || query.accountIds.includes(email.accountId)) &&
          (!query.folderId || email.folderId === query.folderId) &&
          (!query.folderRole || folders.some((folder) => folder.id === email.folderId && folder.role === query.folderRole)) &&
          (query.unread == null || email.isRead === !query.unread) &&
          (query.hasAttachment == null || email.hasAttachment === query.hasAttachment) &&
          (!text || [email.subject, email.preview, email.from.name ?? '', email.from.address].some((field) => field.toLowerCase().includes(text))),
        )
        .sort((a, b) => b.receivedAt.localeCompare(a.receivedAt) || b.id.localeCompare(a.id))
      const offset = query.cursor ? Number(query.cursor) : 0
      const limit = query.limit ?? 50
      const page = items.slice(offset, offset + limit)
      return {
        items: structuredClone(page),
        nextCursor: offset + limit < items.length ? String(offset + limit) : null,
        coverage: { headersComplete: true, bodiesComplete: false, syncedAfter: null, syncedBefore: null, windowCapped: false },
      }
    },
    async getEmail(emailId) {
      const email = emails.find((row) => row.summary.id === emailId)
      if (!email) throw notFound()
      return structuredClone(email)
    },
    async fetchBody(emailId) {
      const email = emails.find((row) => row.summary.id === emailId)
      if (!email) throw notFound()
      return structuredClone(email)
    },
    async setFlag(emailId, flag, wanted) {
      const email = emails.find((row) => row.summary.id === emailId)
      if (!email) throw notFound()
      const key = flag === 'read' ? 'isRead' : 'isStarred'
      if (email.summary[key] === wanted) return 'unchanged' as const
      email.summary[key] = wanted
      // Like the core: a read change moves its folder's unread number in the same
      // step, and only the folder tree is where a client reads that number from.
      if (flag === 'read') {
        const folder = folders.find((row) => row.id === email.summary.folderId)
        if (folder?.countsKnown) folder.unreadCount = Math.max(0, folder.unreadCount + (wanted ? -1 : 1))
      }
      // The second account stands in for a server that cannot keep stars.
      if (flag === 'starred' && email.summary.accountId === accounts[1]?.id) return 'localOnly' as const
      return 'queued' as const
    },
    onFlagWriteBack: () => () => undefined,
    refreshSync: async () => undefined,
    getAvatar: async () => null,
    saveAttachment: async () => 'saved' as const,
    clearImageCache: async () => undefined,
    prepareBody: async () => ({ documentToken: crypto.randomUUID() }),
    releaseBody: async () => undefined,
    imageStatus: async () => [],
    retryImages: async () => ({ documentToken: crypto.randomUUID() }),
    bodyLinks: async (_emailId, documentToken) => ({ documentToken, links: [] }),
    requestLink: async () => { throw notFound() },
    openLink: async () => { throw notFound() },
    onLinkRequest: () => () => undefined,
    // A browser has no notification center or sound device to drive.
    testNotification: async () => undefined,
    previewSound: async () => undefined,
    onNotificationOpen: () => () => undefined,
    async createDraft(request) {
      const accountId = request.accountId ?? (accounts.some((a) => a.id === settings.settings.compose.defaultAccountId) ? settings.settings.compose.defaultAccountId : accounts[0]?.id)
      if (!accountId || !accounts.some((a) => a.id === accountId)) throw notFound()
      const known = draftsByMutation.get(request.clientMutationId)
      if (known && drafts.has(known)) return clone(drafts.get(known)!)
      const draft = newDraft({ accountId, kind: 'new', sourceEmailId: null, to: request.to, cc: request.cc, bcc: request.bcc, subject: request.subject, ...mockDraftBody(request.bodyHtml), inReplyTo: null, references: null })
      draftsByMutation.set(request.clientMutationId, draft.id)
      return clone(draft)
    },
    async createDraftFrom(request) {
      const known = draftsByMutation.get(request.clientMutationId)
      if (known && drafts.has(known)) return clone(drafts.get(known)!)
      const email = emails.find((row) => row.summary.id === request.emailId)
      if (!email) throw notFound()
      const base = email.summary.subject.replace(/^(回复|转发)：/, '')
      const reply = request.kind !== 'forward'
      const original = mockPlainToHtml(email.body?.text ?? '')
      const quoted = reply
        ? `<p></p>${mockPlainToHtml(`${email.summary.from.name ?? email.summary.from.address} 写道：`)}<blockquote>${original}</blockquote>`
        : `<p></p>${mockPlainToHtml(`---------- 转发的邮件 ----------\n发件人：${email.summary.from.address}\n日期：${email.summary.receivedAt}\n主题：${email.summary.subject}\n收件人：${email.to.map((address) => address.address).join(', ')}`)}${original}`
      const draft = newDraft({
        accountId: email.summary.accountId, kind: request.kind, sourceEmailId: email.summary.id,
        to: reply ? [email.summary.from] : [], cc: request.kind === 'replyAll' ? email.cc : [], bcc: [],
        subject: `${reply ? '回复' : '转发'}：${base}`, ...mockDraftBody(quoted), inReplyTo: null, references: null,
      })
      draftsByMutation.set(request.clientMutationId, draft.id)
      return clone(draft)
    },
    async listDrafts(accountId = null) {
      return [...drafts.values()].filter((d) => !accountId || d.accountId === accountId)
        .filter((d) => d.to.length + d.cc.length + d.bcc.length > 0 || d.subject.trim() !== '' || d.bodyText.trim() !== '')
        .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt) || b.id.localeCompare(a.id)).map(summary)
    },
    async getDraft(id) { return clone(requireDraft(id)) },
    async updateDraft(request) {
      const draft = requireDraft(request.id)
      if (request.accountId && !accounts.some((account) => account.id === request.accountId)) throw notFound()
      if (lockedDrafts.has(draft.id)) throw new MailxClientError({ code: 'draftLocked', message: '草稿已进入发送队列，不能修改。', field: null })
      if (draft.revision !== request.expectedRevision) throw new MailxClientError({ code: 'conflict', message: '草稿已被修改，请重新加载后重试。', field: null })
      const { id: _id, expectedRevision: _revision, accountId, ...content } = request
      void _id; void _revision
      Object.assign(draft, content, mockDraftBody(request.bodyHtml), { accountId: accountId ?? draft.accountId, revision: draft.revision + 1, updatedAt: new Date().toISOString() })
      announceDraft({ draftId: draft.id, accountId: draft.accountId, change: 'updated' })
      return clone(draft)
    },
    async deleteDraft(id) {
      const draft = requireDraft(id)
      if (lockedDrafts.has(id)) throw new MailxClientError({ code: 'draftLocked', message: '草稿已进入发送队列，不能修改。', field: null })
      drafts.delete(draft.id)
      announceDraft({ draftId: draft.id, accountId: draft.accountId, change: 'deleted' })
    },
    // The preview has no file system: each "pick" attaches the next sample file, cycling through a few sizes.
    async addAttachments(draftId, expectedRevision) {
      const draft = mutableDraft(draftId, expectedRevision)
      const sample = MOCK_ATTACHMENT_SAMPLES[mockAttachmentCursor++ % MOCK_ATTACHMENT_SAMPLES.length]
      return addMockAttachment(draft, sample)
    },
    async addDroppedAttachments() { return null },
    async removeAttachment({ draftId, attachmentId, expectedRevision }) {
      const draft = mutableDraft(draftId, expectedRevision)
      if (!draft.attachments.some((a) => a.id === attachmentId)) throw notFound()
      draft.attachments = draft.attachments.filter((a) => a.id !== attachmentId)
      return touchDraft(draft)
    },
    async attachmentPolicy() { return { maxTotalBytes: MOCK_ATTACHMENT_LIMIT, maxCount: 20, serverLimitMb: 10 } },
    onFilesDropped: () => () => undefined,
    async queueSend({ draftId, expectedRevision, clientMutationId }) {
      const draft = requireDraft(draftId)
      const again = outbox.byMutation(clientMutationId)
      if (again) return clone(again)
      if (lockedDrafts.has(draftId)) throw new MailxClientError({ code: 'draftLocked', message: '草稿已进入发送队列，不能修改。', field: null })
      if (draft.revision !== expectedRevision) throw new MailxClientError({ code: 'conflict', message: '草稿已被修改，请重新加载后重试。', field: null })
      if (draft.attachments.reduce((sum, a) => sum + a.size, 0) > MOCK_ATTACHMENT_LIMIT) throw new MailxClientError({ code: 'attachmentTooLarge', message: '附件总大小超过 20 MB。', field: null })
      if (draft.attachments.length > 20) throw new MailxClientError({ code: 'tooManyAttachments', message: '附件最多 20 个。', field: null })
      const recipients = [...draft.to, ...draft.cc, ...draft.bcc]
      if (recipients.length === 0) throw new MailxClientError({ code: 'validation', message: '请至少填写一个收件人。', field: 'draft.recipients' })
      lockedDrafts.add(draftId)
      announceDraft({ draftId, accountId: draft.accountId, change: 'locked' })
      return clone(outbox.queue({
        clientMutationId, draftId, accountId: draft.accountId, subject: draft.subject,
        recipients: recipients.slice(0, 3).map((r) => r.address), recipientCount: recipients.length,
        undoMs: settings.settings.compose.undoSendSeconds * 1000,
        simulate: simulatedOutcome(recipients.map((r) => r.address)),
      }))
    },
    listOutbox: async ({ accountId = null, includeAll = false } = {}) => clone(outbox.list(accountId, includeAll)),
    async getOutbox(id) { return clone(outbox.require(id)) },
    async cancelOutbox(id) { return clone(outbox.cancel(id)) },
    async retryOutboxNow(id) { return clone(outbox.retryNow(id)) },
    async resolveUnknown(request) { return clone(outbox.resolve(request.id, request.resolution)) },
    async openComposer(draftId, fresh = false) {
      const draft = clone(requireDraft(draftId))
      options.onOpenComposer?.(draft, fresh)
    },
    closeComposer: async () => undefined,
    cancelComposerClose: async () => undefined,
    onOutbox: (listener) => outbox.subscribe(listener),
    onDrafts(listener) { draftListeners.add(listener); return () => { draftListeners.delete(listener) } },
    onAppClosing: () => () => undefined,
    onAppCloseCancelled: () => () => undefined,
    onComposerCloseRequested: () => () => undefined,
    // The browser preview has no updater: it announces a version but cannot install one.
    getUpdateState: async () => ({ canInstall: false, phase: 'available', version: PREVIEW_UPDATE.version, current: PREVIEW_UPDATE.current, notes: PREVIEW_UPDATE.notes, date: PREVIEW_UPDATE.date }),
    checkForUpdates: async () => ({ canInstall: false, phase: 'available', version: PREVIEW_UPDATE.version, current: PREVIEW_UPDATE.current, notes: PREVIEW_UPDATE.notes, date: PREVIEW_UPDATE.date }),
    downloadUpdate: async () => ({ canInstall: false, phase: 'available', version: PREVIEW_UPDATE.version, current: PREVIEW_UPDATE.current, notes: PREVIEW_UPDATE.notes, date: PREVIEW_UPDATE.date }),
    cancelUpdate: async () => ({ canInstall: false, phase: 'available', version: PREVIEW_UPDATE.version, current: PREVIEW_UPDATE.current, notes: PREVIEW_UPDATE.notes, date: PREVIEW_UPDATE.date }),
    restartToUpdate: async () => { throw new MailxClientError({ code: 'unsupported', message: '当前版本尚不支持此认证方式。', field: null }) },
    openUpdateLink: async (url) => { window.open(url, '_blank', 'noopener,noreferrer') },
    onUpdateState: () => () => undefined,
    getSettings: async () => { readStoredSettings(); return structuredClone(settings) },
    async onSettingsChanged(listener) {
      const own = (event: Event) => listener({ revision: (event as CustomEvent<number>).detail })
      const storage = (event: StorageEvent) => {
        if (event.key !== 'mailx.mock.settings' || !event.newValue) return
        try {
          const next = JSON.parse(event.newValue) as typeof settings
          if (typeof next.revision === 'number' && next.revision > settings.revision &&
              (next.effectiveLocale === 'en' || next.effectiveLocale === 'zh-CN') && next.settings?.general) {
            settings = next
            listener({ revision: settings.revision })
          }
        } catch { /* Ignore an invalid preview cache. */ }
      }
      window.addEventListener('mailx:mock-settings', own)
      window.addEventListener('storage', storage)
      return () => {
        window.removeEventListener('mailx:mock-settings', own)
        window.removeEventListener('storage', storage)
      }
    },
    async updateSettings(next) {
      const save = async () => {
      readStoredSettings()
      if (next.general && !['system', 'zh-CN', 'en'].includes(next.general.language)) {
        throw new MailxClientError({ code: 'validation', field: 'general.language', message: '配置参数无效。' })
      }
      const appearance = next.appearance ?? settings.settings.appearance
      const interfaceFont = parseFontFamilies(appearance.interfaceFont).join(', ')
      const mailFont = parseFontFamilies(appearance.mailFont).join(', ')
      const general = next.general ?? settings.settings.general
      settings = { revision: settings.revision + 1,
        effectiveLocale: general.language === 'system' ? systemLocale() : general.language,
        settings: { ...settings.settings, ...next, appearance: { ...appearance, interfaceFont, mailFont } }, stored: true, recovered: false }
      try { localStorage.setItem('mailx.mock.settings', JSON.stringify(settings)) } catch { /* Optional preview cache. */ }
      window.dispatchEvent(new CustomEvent('mailx:mock-settings', { detail: settings.revision }))
      return structuredClone(settings)
      }
      // Browser tabs share a preview store, just as WebViews share the host store.
      return navigator.locks ? navigator.locks.request('mailx.mock.settings', save) : save()
    },
  }
}

/** Preview-only release notes, shaped like a real `latest.json` section. */
const PREVIEW_UPDATE = {
  // A made-up newer version, so the sample stays valid whatever the app version is.
  version: '99.0.0',
  current: __APP_VERSION__,
  date: Math.floor(Date.parse('2026-11-01T08:00:00Z') / 1000),
  notes: `## 新功能\n\n- 应用内更新：发现新版本后可在设置里查看说明并下载\n- **写信附件** 与 **已发送副本** 的列表展示\n- 修复未命名附件显示为空白的问题\n\n> 预览页面只展示说明，不能安装更新。\n\n\`bun run check\` 通过。`,
} as const

const missing = { code: 'notFound', message: '内容不存在。', field: null } as const

const WORK = '11111111-1111-4111-8111-111111111111'
const PERSONAL = '22222222-2222-4222-8222-222222222222'

function accountFromConfig(id: string, revision: number, config: AccountConfigInput): Account {
  const now = new Date().toISOString()
  const route: RouteConfig =
    config.route.kind === 'custom'
      ? {
          ...config.route,
          authentication: config.route.authentication && {
            username: config.route.authentication.username,
            passwordConfigured: true,
            trust: 'trusted',
          },
        }
      : config.route
  const separate = config.authentication.kind === 'password' && config.authentication.mode === 'separate'
  const auth = (suffix: string) => ({ id: `${id}-${suffix}`, generation: 1, passwordConfigured: true, trust: 'trusted' as const })
  return {
    id,
    revision,
    displayName: config.displayName,
    color: config.color ?? null,
    useProviderIcon: config.useProviderIcon ?? true,
    senderName: config.senderName?.trim() || null,
    emailAddress: config.emailAddress,
    providerId: config.providerId,
    mailboxSourceId: `mock-${id}`,
    imap: config.imap,
    smtp: config.smtp,
    authentication: separate
      ? { mode: 'separate', imap: auth('imap'), smtp: auth('smtp') }
      : { mode: 'shared', imap: auth('shared'), smtp: auth('shared') },
    route,
    routeRevision: 1,
    defaultRouteRevision: null,
    connectionStatus: 'unverified',
    createdAt: now,
    updatedAt: now,
  }
}

const DEFAULT_ACCOUNTS: Account[] = [
  accountFromConfig(WORK, 1, {
    displayName: '工作邮箱',
    useProviderIcon: true,
    color: '#2563eb',
    senderName: null,
    emailAddress: 'lin.wei@example.com',
    providerId: null,
    imap: { host: 'imap.example.com', port: 993, tls: 'implicit', username: 'lin.wei@example.com' },
    smtp: { host: 'smtp.example.com', port: 465, tls: 'implicit', username: 'lin.wei@example.com' },
    authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } },
    route: { kind: 'direct' },
  }),
  accountFromConfig(PERSONAL, 1, {
    displayName: '个人邮箱',
    useProviderIcon: true,
    color: '#16a34a',
    senderName: null,
    emailAddress: 'linwei.home@example.org',
    providerId: null,
    imap: { host: 'imap.example.org', port: 993, tls: 'implicit', username: 'linwei.home@example.org' },
    smtp: { host: 'smtp.example.org', port: 587, tls: 'startTls', username: 'linwei.home@example.org' },
    authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } },
    route: { kind: 'direct' },
  }),
]

const FOLDER_NAMES: [FolderRole, string][] = [
  ['inbox', '收件箱'],
  ['drafts', '草稿'],
  ['sent', '已发送'],
  ['archive', '归档'],
  ['junk', '垃圾邮件'],
  ['trash', '已删除'],
]

const folderId = (accountId: string, role: FolderRole) => `${accountId}:${role}`

/** User folders by account: `[path, name, parent path, selectable]`; `selectable: false` mimics IMAP `\\Noselect`. */
const DEFAULT_CUSTOM_FOLDERS: CustomFolders = {
  [WORK]: [
    ['projects', '项目', null, true],
    ['projects/mailx', 'MailX', 'projects', true],
    ['projects/q3', '第三季度', 'projects', true],
    ['clients', '客户', null, false],
    ['clients/east', '华东区', 'clients', true],
  ],
  [PERSONAL]: [['travel', '旅行', null, true]],
}

function buildFolders(accountId: string, emails: EmailDetail[], customFolders: CustomFolders): Folder[] {
  const custom = (customFolders[accountId] ?? []).map(
    ([path, name, parent, selectable]) => [`${accountId}:${path}`, name, 'other', parent && `${accountId}:${parent}`, selectable] as const,
  )
  const system = FOLDER_NAMES.map(([role, name]) => [folderId(accountId, role), name, role, null, true] as const)
  return [...system, ...custom].map(([id, name, role, parentId, selectable]) => {
    const inFolder = emails.filter((email) => email.summary.folderId === id)
    return {
      id,
      accountId,
      parentId,
      name,
      role,
      selectable,
      countsKnown: true,
      unreadCount: inFolder.filter((email) => !email.summary.isRead).length,
      totalCount: inFolder.length,
    }
  })
}

export type CustomFolders = Record<string, [string, string, string | null, boolean][]>

export type Seed = {
  account: string
  role?: FolderRole
  /** Path of a user folder from `CUSTOM_FOLDERS`; overrides `role`. */
  folder?: string
  from: MailAddress
  subject: string
  text: string
  hoursAgo: number
  read?: boolean
  starred?: boolean
  attachments?: [string, string, number][]
}

const ME_WORK = { name: '林伟', address: 'lin.wei@example.com' }
const ME_HOME = { name: '林伟', address: 'linwei.home@example.org' }

const DEFAULT_SEEDS: Seed[] = [
  {
    account: WORK,
    from: { name: '陈晓', address: 'chen.xiao@example.com' },
    subject: '第三季度产品路线图评审',
    text: '大家好，\n\n周四下午 3 点在 5 楼会议室评审第三季度路线图，请提前看一下附件里的草稿，重点关注同步模块的排期。\n\n如果时间冲突请回复我。\n\n陈晓',
    hoursAgo: 0.4,
    attachments: [['Q3 路线图草稿.pdf', 'application/pdf', 1_284_000]],
  },
  {
    account: WORK,
    from: { name: 'GitHub', address: 'notifications@github.example' },
    subject: '[MailX] Pull request #1: feat(desktop): add native-feeling custom title bar',
    text: '有新的评论等待你回复。\n\n> 在 Windows 上关闭按钮的悬停颜色看起来正确，Linux 还需要测试。',
    hoursAgo: 1.5,
  },
  {
    account: WORK,
    from: { name: '王芳', address: 'wang.fang@example.com' },
    subject: '回复：发票报销流程变更',
    text: '收到，我下周一前把十月份的发票整理好交给财务。新流程里电子发票是不是不需要再打印纸质版了？',
    hoursAgo: 3,
    read: true,
    starred: true,
  },
  {
    account: WORK,
    from: { name: '运维通知', address: 'ops@example.com' },
    subject: '周六凌晨数据库维护窗口',
    text: '本周六 02:00–04:00 将对主数据库进行版本升级，期间内部系统只读。请提前安排好需要写入的任务。',
    hoursAgo: 9,
    read: true,
  },
  {
    account: WORK,
    from: { name: '李娜', address: 'li.na@example.com' },
    folder: 'projects/mailx',
    subject: '设计稿：邮件列表的紧凑模式',
    text: '附上紧凑模式的两版设计，A 版本每行显示两行摘要，B 版本只显示主题。你更倾向哪一版？',
    hoursAgo: 26,
    attachments: [
      ['紧凑模式-A.png', 'image/png', 412_000],
      ['紧凑模式-B.png', 'image/png', 398_000],
    ],
  },
  {
    account: WORK,
    from: { name: '张强', address: 'zhang.qiang@example.com' },
    folder: 'projects/mailx',
    subject: '关于 IMAP IDLE 的兼容性测试结果',
    text: '测试了 6 家服务商，其中 2 家在空闲 29 分钟后会主动断开 IDLE，需要我们提前重连。详细表格见附件。',
    hoursAgo: 50,
    read: true,
    attachments: [['IDLE 兼容性.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', 86_000]],
  },
  {
    account: WORK,
    role: 'sent',
    from: ME_WORK,
    subject: '回复：第二季度复盘',
    text: '谢谢大家，复盘纪要已经整理到共享文档，行动项负责人请在周五前更新进度。',
    hoursAgo: 75,
    read: true,
  },
  {
    account: WORK,
    role: 'drafts',
    from: ME_WORK,
    subject: '（无主题）',
    text: '关于下个版本的发布计划，我想先确认一下……',
    hoursAgo: 4,
    read: true,
  },
  {
    account: WORK,
    role: 'archive',
    from: { name: '人力资源部', address: 'hr@example.com' },
    subject: '年度体检预约通知',
    text: '年度体检预约通道已开放，请在 11 月 15 日前完成预约。',
    hoursAgo: 240,
    read: true,
  },
  {
    account: PERSONAL,
    from: { name: '中国国际航空', address: 'service@airline.example' },
    folder: 'travel',
    subject: '您的行程单：北京 — 上海 11月2日',
    text: '尊敬的旅客，您的航班 CA1501 将于 11 月 2 日 08:00 起飞，请提前 90 分钟到达机场办理值机。',
    hoursAgo: 2,
    attachments: [['电子行程单.pdf', 'application/pdf', 152_000]],
  },
  {
    account: WORK,
    folder: 'clients/east',
    from: { name: '赵敏', address: 'zhao.min@client.example' },
    subject: '合同续签的几点意见',
    text: '附件是我们法务对续签合同的修改意见，主要集中在服务等级和数据存放地两条，方便的话周三电话沟通。',
    hoursAgo: 20,
  },
  {
    account: PERSONAL,
    from: { name: '妈妈', address: 'mom@example.net' },
    subject: '周末回家吃饭吗',
    text: '周末有空回来吃饭吧，你爸买了条鱼。记得带上上次说的那本书。',
    hoursAgo: 6,
  },
  {
    account: PERSONAL,
    from: { name: '读书会', address: 'club@example.org' },
    subject: '十一月书单投票',
    text: '本月候选书目：《三体》《置身事内》《人类简史》。点击下面的链接投票，周日截止。',
    hoursAgo: 30,
    read: true,
  },
  {
    account: PERSONAL,
    role: 'junk',
    from: { name: '中奖通知', address: 'winner@prize.example' },
    subject: '恭喜您获得 iPhone 一部！！！',
    text: '点击链接领取奖品。',
    hoursAgo: 12,
  },
  {
    account: PERSONAL,
    from: { name: '物业服务中心', address: 'service@property.example' },
    subject: '停水通知',
    text: '因管道检修，11 月 1 日 9:00–15:00 小区 3、4 号楼停水，请提前储水。',
    hoursAgo: 80,
    read: true,
    starred: true,
  },
]

type MockDataset = {
  accounts: Account[]
  seeds: Seed[]
  customFolders: CustomFolders
  /** The address each account's mail is delivered to. */
  me: Record<string, MailAddress>
  /** Whether the first seed has an HTML body (the browser preview shows a fixed document for it). */
  htmlFirst: boolean
}

const DEFAULT_DATASET: MockDataset = {
  accounts: DEFAULT_ACCOUNTS,
  seeds: DEFAULT_SEEDS,
  customFolders: DEFAULT_CUSTOM_FOLDERS,
  me: { [WORK]: ME_WORK, [PERSONAL]: ME_HOME },
  htmlFirst: true,
}

/** Fictitious data for README screenshots, selected with `?seed=showcase`; see `mock-showcase.ts`. */
function showcaseDataset(): MockDataset {
  return {
    accounts: SHOWCASE_DATASET.accounts.map(({ id, config }) => accountFromConfig(id, 1, config)),
    seeds: SHOWCASE_DATASET.seeds,
    customFolders: SHOWCASE_DATASET.customFolders,
    me: SHOWCASE_DATASET.me,
    htmlFirst: false,
  }
}

// Relative to load time so the list always looks recent.
const BASE_TIME = Date.now()

function buildEmails({ seeds, me: addresses, htmlFirst }: MockDataset): EmailDetail[] {
  return seeds.map((seed, index) => {
    const id = `mock-email-${index + 1}`
    const me = addresses[seed.account]
    const sent = seed.role === 'sent' || seed.role === 'drafts'
    const summary: EmailSummary = {
      id,
      accountId: seed.account,
      folderId: seed.folder ? `${seed.account}:${seed.folder}` : folderId(seed.account, seed.role ?? 'inbox'),
      subject: seed.subject,
      from: seed.from,
      preview: seed.text.replace(/\s+/g, ' ').slice(0, 120),
      receivedAt: new Date(BASE_TIME - seed.hoursAgo * 3_600_000).toISOString(),
      isRead: seed.read ?? false,
      isStarred: seed.starred ?? false,
      hasAttachment: (seed.attachments?.length ?? 0) > 0,
      bodyState: 'ready',
      pendingChanges: false,
    }
    return {
      summary,
      to: sent ? [{ name: '团队', address: 'team@example.com' }] : [me],
      cc: [],
      replyTo: [],
      body: { text: seed.text, hasHtml: htmlFirst && index === 0, truncated: false, remoteImagesBlocked: htmlFirst && index === 0 ? 1 : 0 },
      bodyFailure: null,
      attachments: (seed.attachments ?? []).map(([filename, mimeType, size], position) => ({
        id: `${id}-attachment-${position + 1}`,
        filename,
        mimeType,
        size,
        isInline: false,
      })),
    }
  })
}

/** Browser preview only (`?seed=outbox`): one plain draft plus sends that end failed, unknown and retrying. */
export async function seedPreviewOutbox(client: MailxClient) {
  const make = async (to: string, subject: string, bodyText: string) => {
    const draft = await client.createDraft({
      accountId: null, to: [{ name: null, address: to }], cc: [], bcc: [], subject, bodyHtml: mockPlainToHtml(bodyText), clientMutationId: crypto.randomUUID(),
    })
    return draft
  }
  await make('team@example.com', '周会纪要（草稿）', '各位好，\n\n本周的进展如下……')
  for (const [to, subject] of [['fail@example.com', '报价单'], ['unknown@example.com', '合同终稿'], ['retry@example.com', '会议邀请']] as const) {
    const draft = await make(to, subject, '正文')
    await client.queueSend({ draftId: draft.id, expectedRevision: draft.revision, clientMutationId: crypto.randomUUID() })
  }
}

/**
 * Browser-mode only, never a product feature: the recipients decide how the simulated
 * send ends so the failure UI can be exercised without a server.
 *   fail@...     the server rejects that recipient (state `failed`)
 *   unknown@...  no final reply arrives (state `unknown`)
 *   retry@...    the first attempt fails before any data and is retried after 30 s
 *                (state stays `queued` with a next attempt; "立即重试" then succeeds)
 * Anything else is accepted.
 */
type Simulated = { kind: 'accept' } | { kind: 'fail'; address: string } | { kind: 'unknown' } | { kind: 'retry' }
function simulatedOutcome(addresses: string[]): Simulated {
  const find = (prefix: string) => addresses.find((address) => address.toLowerCase().startsWith(prefix))
  const failing = find('fail@')
  if (failing) return { kind: 'fail', address: failing }
  if (find('unknown@')) return { kind: 'unknown' }
  if (find('retry@')) return { kind: 'retry' }
  return { kind: 'accept' }
}

type QueueInput = {
  clientMutationId: string
  draftId: string
  accountId: string
  subject: string
  recipients: string[]
  recipientCount: number
  undoMs: number
  simulate?: Simulated
}

/**
 * The browser-mode send queue: queued -> sending -> accepted on timers. It only
 * imitates the core's state machine for designing the UI. Nothing is sent, and
 * the replies and message ids are visibly synthetic.
 */
function createMockOutbox(stepMs: number) {
  const items = new Map<string, OutboxItem>()
  const byMutation = new Map<string, string>()
  const listeners = new Set<(change: OutboxChanged) => void>()
  const timers = new Map<string, ReturnType<typeof setTimeout>>()
  const plans = new Map<string, Simulated>()
  const api = {
    onAccepted: undefined as ((item: OutboxItem) => void) | undefined,
    onCanceled: undefined as ((item: OutboxItem) => void) | undefined,
    onFailed: undefined as ((item: OutboxItem) => void) | undefined,
    byMutation: (id: string) => (byMutation.has(id) ? items.get(byMutation.get(id)!) : undefined),
    require(id: string) {
      const item = items.get(id)
      if (!item) throw new MailxClientError(missing)
      return item
    },
    queue(input: QueueInput) {
      const now = new Date()
      const id = crypto.randomUUID()
      const item: OutboxItem = {
        id, draftId: input.draftId, accountId: input.accountId, messageId: `<mock-${id}@mock.invalid>`, subject: input.subject,
        recipients: input.recipients, recipientCount: input.recipientCount, state: 'queued',
        notBefore: new Date(now.getTime() + input.undoMs).toISOString(), dataStartedAt: null, attempts: 0, nextAttemptAt: null,
        firstQueuedAt: now.toISOString(), error: null, acceptedAt: null, serverReply: null, serverCopyMessageId: null,
        resolution: null, supersededBy: null, createdAt: now.toISOString(), updatedAt: now.toISOString(),
      }
      items.set(id, item)
      plans.set(id, input.simulate ?? { kind: 'accept' })
      byMutation.set(input.clientMutationId, id)
      announce(item)
      timers.set(id, setTimeout(() => begin(id), input.undoMs + stepMs))
      return item
    },
    list(accountId: string | null, includeAll: boolean) {
      return [...items.values()]
        .filter((item) => (!accountId || item.accountId === accountId) &&
          (includeAll || item.state === 'queued' || item.state === 'sending' || item.state === 'failed' || (item.state === 'unknown' && !item.resolution)))
        .sort((a, b) => b.createdAt.localeCompare(a.createdAt) || b.id.localeCompare(a.id))
    },
    cancel(id: string) {
      const item = api.require(id)
      if (item.state === 'sending') throw new MailxClientError({ code: 'alreadySending', message: '已经开始发送，无法撤销。', field: null })
      if (item.state !== 'queued') throw new MailxClientError({ code: 'conflict', message: '当前状态不能撤销。', field: null })
      clearTimeout(timers.get(id))
      set(item, 'canceled')
      api.onCanceled?.(item)
      return item
    },
    retryNow(id: string) {
      const item = api.require(id)
      if (item.state !== 'queued' || !item.nextAttemptAt) throw new MailxClientError({ code: 'conflict', message: '当前状态不能重试。', field: null })
      clearTimeout(timers.get(id))
      begin(id)
      return item
    },
    resolve(id: string, resolution: 'markSent' | 'resend') {
      const item = api.require(id)
      if (item.state !== 'unknown' || item.resolution) throw new MailxClientError({ code: 'conflict', message: '当前状态不需要确认。', field: null })
      if (resolution === 'markSent') {
        item.resolution = 'markedSent'
        item.acceptedAt = new Date().toISOString()
        set(item, 'accepted')
        api.onAccepted?.(item)
        return item
      }
      item.resolution = 'resent'
      const next = api.queue({
        clientMutationId: crypto.randomUUID(), draftId: item.draftId ?? '', accountId: item.accountId, subject: item.subject,
        recipients: item.recipients, recipientCount: item.recipientCount, undoMs: 0, simulate: { kind: 'accept' },
      })
      next.messageId = item.messageId
      item.supersededBy = next.id
      return next
    },
    subscribe(listener: (change: OutboxChanged) => void) {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    removeAccount(accountId: string) {
      const affected = [...items.values()].filter((item) => item.accountId === accountId)
      if (affected.some((item) => item.state === 'sending')) throw new MailxClientError({ code: 'alreadySending', message: '', field: null })
      for (const item of affected) {
        clearTimeout(timers.get(item.id))
        timers.delete(item.id)
        plans.delete(item.id)
        items.delete(item.id)
      }
      for (const [mutation, itemId] of byMutation) if (!items.has(itemId)) byMutation.delete(mutation)
    },
  }

  function announce(item: OutboxItem) {
    const change: OutboxChanged = {
      accountId: item.accountId, outboxId: item.id, draftId: item.draftId, state: item.state, attempts: item.attempts,
      nextAttemptAt: item.nextAttemptAt, errorReason: item.error?.reason ?? null,
    }
    listeners.forEach((listener) => listener(change))
  }
  function set(item: OutboxItem, state: OutboxState) {
    item.state = state
    item.updatedAt = new Date().toISOString()
    announce(item)
  }
  function begin(id: string) {
    const item = items.get(id)
    if (!item || item.state !== 'queued') return
    item.attempts += 1
    item.nextAttemptAt = null
    const plan = plans.get(id) ?? { kind: 'accept' }
    if (plan.kind === 'retry' && item.attempts === 1) {
      // Before any data: back to the queue with a retry scheduled, like a refused connection.
      item.error = { reason: 'connect', smtpCode: null, text: 'mock: connection refused (simulated)', rejected: [] }
      item.nextAttemptAt = new Date(Date.now() + 30_000).toISOString()
      set(item, 'queued')
      timers.set(id, setTimeout(() => begin(id), 30_000))
      return
    }
    item.dataStartedAt = new Date().toISOString()
    set(item, 'sending')
    timers.set(id, setTimeout(() => {
      if (plan.kind === 'fail') {
        item.error = { reason: 'recipientsRejected', smtpCode: 550, text: null, rejected: [{ address: plan.address, code: 550, text: 'mock: mailbox unavailable (simulated)' }] }
        item.dataStartedAt = null
        set(item, 'failed')
        api.onFailed?.(item)
        return
      }
      if (plan.kind === 'unknown') {
        item.error = { reason: 'unknown', smtpCode: null, text: 'connectionLost', rejected: [] }
        set(item, 'unknown')
        return
      }
      item.error = null
      item.acceptedAt = new Date().toISOString()
      item.serverReply = { code: 250, text: 'mock: accepted (simulated, nothing was sent)' }
      set(item, 'accepted')
      api.onAccepted?.(item)
    }, stepMs))
  }
  return api
}
