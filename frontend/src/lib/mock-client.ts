import type {
  EmailListQuery,
  Account,
  AccountRemoval,
  AccountConfigInput,
  OAuthSessionHandle,
  OAuthSignInResult,
  RouteInput,
  Draft,
  InlineImage,
  DraftChanged,
  DraftSummary,
  OutboxChanged,
  OutboxItem,
  OutboxState,
  ConversationPage,
  EmailDetail,
  EmailPreviews,
  EmailSummary,
  Folder,
  FolderRole,
  MailAddress,
  MailStateChanged,
  RouteConfig,
  TranslationProviderId,
  TranslationStatus,
} from '@/generated/accounts'
import { cachedAppearance, parseFontFamilies } from '@/lib/appearance'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { matchEmail, parseSearchTerms } from '@/lib/mock-search'
import { DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'
import { SHOWCASE_DATASET } from '@/lib/mock-showcase'
import type { McpClientConfig, McpStatus } from '@/features/mcp/types'
import { mockDraftBody, mockPlainToHtml } from './mock-draft-body'
import { cachedLanguage, systemLocale } from '@/i18n/locale'

function normalizedAccountOrder(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.length > 256) return null
  const order = new Set<string>()
  for (const entry of value) {
    if (typeof entry !== 'string') return null
    const id = entry.trim().replace(/^urn:uuid:/, '').replace(/^\{(.*)\}$/, '$1')
    if (!/^(?:[\da-f]{32}|[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})$/i.test(id)) return null
    const hex = id.replaceAll('-', '').toLowerCase()
    order.add(`${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`)
  }
  return [...order]
}

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
  const sentCopiesListeners = new Set<(change: import('@/generated/accounts').SentCopiesChanged) => void>()
  const folders = accounts.flatMap((account) => buildFolders(account.id, emails, dataset.customFolders))
  let settings: import('@/generated/accounts').SettingsSnapshot = { effectiveLocale: cachedLanguage().effectiveLocale, revision: 1, settings: { general: { language: cachedLanguage().language, autoUpdate: true, closeBehavior: 'ask', showMenuBarIcon: true, silentStart: false }, appearance: cachedAppearance(), security: { remoteImages: 'ask' as const, senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, compose: { defaultAccountId: '', undoSendSeconds: 0 }, mcp: { enabled: false }, translation: { provider: null, microsoftRegion: '', route: { kind: 'direct' }, targetLanguage: 'auto', suggest: true, neverSuggest: [], alwaysTranslate: [], includeQuotes: false, consentedProvider: null } }, stored: true, recovered: false }

  function readStoredSettings() {
    try {
      const value = JSON.parse(localStorage.getItem('mailx.mock.settings') ?? 'null') as typeof settings | null
      if (value && Number.isSafeInteger(value.revision) && value.revision >= settings.revision &&
          ['system', 'zh-CN', 'en'].includes(value.settings?.general?.language) &&
          ['en', 'zh-CN'].includes(value.effectiveLocale) &&
          ['appearance', 'security', 'layout', 'notifications', 'compose'].every((key) =>
            typeof value.settings[key as keyof typeof value.settings] === 'object')) {
        settings = { ...value, settings: { ...value.settings, translation: value.settings.translation ?? settings.settings.translation, layout: { ...DEFAULT_LAYOUT, ...value.settings.layout,
          accountOrder: normalizedAccountOrder(value.settings.layout.accountOrder ?? []) ?? [] } } }
      }
    } catch { /* The browser preview cache is optional. */ }
  }
  readStoredSettings()

  const notFound = () => new MailxClientError(missing)
  const sendStepMs = options.sendStepMs ?? 600
  const outbox = createMockOutbox(sendStepMs)
  const drafts = new Map<string, Draft>()
  const draftsByMutation = new Map<string, string>()
  const previewListeners = new Set<(previews: EmailPreviews) => void>()
  let previewRequest = 0
  const draftListeners = new Set<(change: DraftChanged) => void>()
  let mcpStatus: McpStatus = { state: 'off' }
  const mcpListeners = new Set<(status: McpStatus) => void>()
  const lockedDrafts = new Set<string>()
  const removedDrafts = new Set<string>()
  const removals = new Map<string, { revision: number; result: AccountRemoval }>()
  const removalListeners = new Set<(removal: AccountRemoval) => void>()
  const oauthSessions = createOAuthSessions()
  const announceDraft = (change: DraftChanged) => draftListeners.forEach((listener) => listener(structuredClone(change)))
  const clone = <T,>(value: T) => structuredClone(value)
  const MOCK_DELETE_UNDO_MS = 5000
  const pendingDeletions = new Map<string, { timer: ReturnType<typeof setTimeout>; emails: typeof emails }>()
  const archiveReceipts = new Map<string, { emailIds: string[]; before: boolean; expires: number }>()
  const mailStateListeners = new Set<(change: MailStateChanged) => void>()

  const MOCK_ATTACHMENT_LIMIT = 20 * 1024 * 1024
  const MOCK_TRANSLATION_KEYS: TranslationProviderId[] = ['deepl', 'microsoft', 'google']
  const MOCK_TRANSLATION_MS = 900
  // Keys live only in memory, like the real ones live only in the secret store: nothing reads them back out.
  const translationSecrets = new Map<TranslationProviderId, string>()
  let proxyPassword: string | null = null
  const translations = new Map<string, { provider: TranslationProviderId; sourceLanguage: string | null; subject: string }>()
  const translationRuns = new Map<string, { cancel: () => void }>()
  const translationKey = (emailId: string, target: string) => `${emailId}|${target}|${settings.settings.translation.includeQuotes}`
  const translationFailure = (code: Extract<import('@/generated/accounts').ErrorCode, `translation${string}`>) =>
    new MailxClientError({ code, message: '翻译未完成。', field: null })
  const resolvedTarget = () => {
    const { targetLanguage } = settings.settings.translation
    return targetLanguage === 'auto' ? (settings.effectiveLocale === 'zh-CN' ? 'zh-Hans' : 'en') : targetLanguage
  }
  function translationStatusOf(emailId: string, explicitTarget?: string): TranslationStatus {
    const email = emails.find((row) => row.summary.id === emailId)
    if (!email) throw notFound()
    const config = settings.settings.translation
    const target = explicitTarget ?? resolvedTarget()
    const detected = detectLanguage(email.body?.text ?? '')
    const keyed = config.provider !== null && translationSecrets.has(config.provider)
    const reliable = detected !== null
    const suggested = keyed && config.suggest && reliable && detected !== target.split('-')[0] && !config.neverSuggest.includes(detected)
    const found = translations.get(translationKey(emailId, target))
    return {
      emailId, targetLanguage: target, detectedLanguage: detected, detectionReliable: reliable, suggested,
      autoTranslate: suggested && config.consentedProvider === config.provider && config.alwaysTranslate.includes(detected),
      translation: found ? { ...found, partial: false } : null,
    }
  }
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
    draft.attachments.push({ id: crypto.randomUUID(), fileName, mediaType, size, origin: 'user', inline: false, contentId: null })
    return touchDraft(draft)
  }

  // Inline images keep their bytes so the preview can hand them back.
  const inlineBytes = new Map<string, Uint8Array>()
  const MOCK_PIXEL = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=='), (c) => c.charCodeAt(0))
  function addMockInline(draft: Draft, fileName: string, bytes: Uint8Array) {
    const hex = Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('')
    const image: InlineImage = { attachmentId: crypto.randomUUID(), contentId: `${hex}@example.com`, fileName }
    draft.attachments.push({ id: image.attachmentId, fileName, mediaType: 'image/png', size: bytes.byteLength, origin: 'user', inline: true, contentId: image.contentId })
    inlineBytes.set(image.attachmentId, bytes)
    return image
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
      id, accountId: accounts[0]?.id ?? '', kind: 'new', origin: 'user', sourceEmailId: null, to: [], cc: [], bcc: [], subject: '', bodyHtml: '<p></p>', bodyText: '',
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
    id: draft.id, accountId: draft.accountId, kind: draft.kind, origin: draft.origin, sourceEmailId: draft.sourceEmailId, to: draft.to,
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

  // Local archive: only inbox mail, all or nothing; the server is not involved.
  // With `partial`, ids that are not visible inbox mail are skipped and reported instead.
  function changeArchive(emailIds: string[], archived: boolean, undo = false, partial = false) {
    const requested = [...new Set(emailIds)]
    if (requested.length === 0 || requested.length > 500) throw new MailxClientError({ code: 'validation', message: '一次只能处理 1 到 500 封邮件。', field: null })
    const find = (id: string) => emails.find((row) => row.summary.id === id && !row.summary.localOnly && folders.find((folder) => folder.id === row.summary.folderId)?.role === 'inbox')
    const skippedIds = partial ? requested.filter((id) => !find(id)) : []
    const ids = requested.filter((id) => !skippedIds.includes(id))
    if (ids.length === 0) throw notFound()
    const picked = ids.map(find)
    if (picked.some((email) => !email)) throw notFound()
    for (const [token, receipt] of archiveReceipts) {
      if (receipt.expires <= Date.now() || receipt.emailIds.some((id) => ids.includes(id))) archiveReceipts.delete(token)
    }
    const changed: EmailDetail[] = []
    for (const email of picked) {
      if (!email || email.summary.archived === archived) continue
      email.summary.archived = archived
      changed.push(email)
      const folder = folders.find((row) => row.id === email.summary.folderId)
      if (!folder) continue
      const sign = archived ? 1 : -1
      folder.archivedCount += sign
      if (!email.summary.isRead) {
        folder.archivedUnreadCount += sign
        folder.unreadCount = Math.max(0, folder.unreadCount - sign)
      }
    }
    const undoToken = changed.length > 0 && !undo ? crypto.randomUUID() : null
    if (undoToken) {
      if (archiveReceipts.size >= 100) archiveReceipts.delete(archiveReceipts.keys().next().value!)
      archiveReceipts.set(undoToken, { emailIds: changed.map((email) => email.summary.id), before: !archived, expires: Date.now() + 10 * 60 * 1000 })
    }
    if (changed.length > 0) {
      const event: MailStateChanged = {
        accountIds: [...new Set(changed.map((email) => email.summary.accountId))],
        emailIds: changed.map((email) => email.summary.id), source: 'user',
        kind: undo ? 'undoArchive' : archived ? 'archive' : 'unarchive', changedCount: changed.length, undoToken,
      }
      for (const listener of mailStateListeners) listener(event)
    }
    return { processedCount: ids.length, changedCount: changed.length, undoToken, skippedIds }
  }

  function applyFlag(emailId: string, flag: 'read' | 'starred', wanted: boolean) {
    const email = emails.find((row) => row.summary.id === emailId)
    if (!email) throw notFound()
    if (email.summary.localOnly) throw new MailxClientError({ code: 'validation', message: '本机副本不能更改标记。', field: null })
    const key = flag === 'read' ? 'isRead' : 'isStarred'
    if (email.summary[key] === wanted) return 'unchanged' as const
    email.summary[key] = wanted
    // Like the core: a read change moves its folder's unread number in the same
    // step, and only the folder tree is where a client reads that number from.
    if (flag === 'read') {
      const folder = folders.find((row) => row.id === email.summary.folderId)
      if (email.summary.archived && folder) {
        // Archived unread mail counts in the archived views, not in the inbox number.
        folder.archivedUnreadCount = Math.max(0, folder.archivedUnreadCount + (wanted ? -1 : 1))
      } else if (folder?.countsKnown) folder.unreadCount = Math.max(0, folder.unreadCount + (wanted ? -1 : 1))
    }
    // The second account stands in for a server that cannot keep stars.
    if (flag === 'starred' && email.summary.accountId === accounts[1]?.id) return 'localOnly' as const
    return 'queued' as const
  }

  /** Aggregated sender addresses (lowercase) and when each rule was made. */
  const contactRules = new Map<string, string>()

  /** Like the core: trimmed, non-empty, at most 320 characters, with an `@` and no control characters. */
  function senderAddress(value: string, field: string) {
    const address = value.trim()
    // eslint-disable-next-line no-control-regex
    if (!address || address.length > 320 || !address.includes('@') || /[\u0000-\u001f\u007f-\u009f]/.test(address)) {
      throw new MailxClientError({ code: 'validation', message: '', field })
    }
    return address.toLowerCase()
  }

  /**
   * Every message a list query matches, newest first, as `emails_list` orders them:
   * `items` are the list rows (aggregated senders collapsed into group rows when the
   * query groups them) and `individual` the messages that are listed as themselves.
   */
  function matchingEmails(query: EmailListQuery) {
    const terms = query.text?.trim() ? parseSearchTerms(query.text) : []
    if (!terms) throw new MailxClientError({ code: 'validation', message: '搜索词过多或过长。', field: 'text' })
    const fromAddress = query.fromAddress == null ? null : senderAddress(query.fromAddress, 'fromAddress')
    const archiveScope = query.archive ?? (terms.length > 0 ? 'include' : 'exclude')
    const inScope = emails.filter((email) =>
      (!query.accountIds || query.accountIds.includes(email.summary.accountId)) &&
      (!query.folderId || email.summary.folderId === query.folderId) &&
      (!query.folderRole || folders.some((folder) => folder.id === email.summary.folderId && folder.role === query.folderRole)) &&
      !(query.excludeFolderRoles ?? []).some((role) => folders.some((folder) => folder.id === email.summary.folderId && folder.role === role)) &&
      // Like the core: archived mail only shows in the archived views and in searches.
      (archiveScope === 'include' || (archiveScope === 'only') === email.summary.archived) &&
      (query.unread == null || email.summary.isRead === !query.unread) &&
      (query.starred == null || email.summary.isStarred === query.starred) &&
      (query.hasAttachment == null || email.summary.hasAttachment === query.hasAttachment) &&
      // A local Sent copy's sender is the account's own address.
      (fromAddress == null || (email.summary.localOnly
        ? accounts.find((account) => account.id === email.summary.accountId)?.emailAddress.toLowerCase() === fromAddress
        : email.summary.from.address.toLowerCase() === fromAddress)),
    )
    const newestFirst = <T extends { receivedAt: string; id: string }>(a: T, b: T) => b.receivedAt.localeCompare(a.receivedAt) || b.id.localeCompare(a.id)
    const matched = inScope
      .flatMap((email) => {
        if (terms.length === 0) return [{ ...email.summary, matchSnippet: null }]
        const { matched, snippet } = matchEmail(email, terms)
        return matched ? [{ ...email.summary, matchSnippet: snippet }] : []
      })
      .sort(newestFirst)
    // Never in a text search or a single-sender view; local Sent copies are never grouped.
    const grouping = query.groupSenders && terms.length === 0 && fromAddress == null
    const isGrouped = (email: EmailSummary) => grouping && !email.localOnly && contactRules.has(email.from.address.toLowerCase())
    const groups = new Map<string, { total: number; unread: number }>()
    for (const email of matched) {
      if (!isGrouped(email)) continue
      const address = email.from.address.toLowerCase()
      const group = groups.get(address) ?? { total: 0, unread: 0 }
      group.total += 1
      if (!email.isRead) group.unread += 1
      groups.set(address, group)
    }
    const shown = new Set<string>()
    const items = matched.filter((email) => {
      if (!isGrouped(email)) return true
      const address = email.from.address.toLowerCase()
      if (shown.has(address)) return false
      shown.add(address)
      return true
    }).map((email) => isGrouped(email) ? { ...email, senderGroup: { address: email.from.address.toLowerCase(), ...groups.get(email.from.address.toLowerCase())! } } : email)
    return { inScope, items, individual: matched.filter((email) => !isGrouped(email)) }
  }

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
      return { accountId, revision: account.revision, imap: check((account.authentication.kind === 'password' ? account.authentication.imap.passwordConfigured : account.authentication.imap.credentialConfigured), account.authentication.imap.trust), smtp: check((account.authentication.kind === 'password' ? account.authentication.smtp.passwordConfigured : account.authentication.smtp.credentialConfigured), account.authentication.smtp.trust) }
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
          : providerId === 'icloud' ? 'https://support.apple.com/en-us/102654'
            : providerId === 'gmail' ? 'https://support.google.com/accounts/answer/185833'
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
    async beginOAuthAccount({ providerId, route }) {
      if (!OAUTH_PROVIDER_IDS.includes(providerId)) throw new MailxClientError({ code: 'unsupported', message: '', field: null })
      return oauthSessions.begin({ kind: 'create', providerId, route, hint: mockSignInAddress(providerId), mailbox: null, attempts: 0 })
    },
    async reauthorizeOAuthAccount({ providerId, accountId, expectedRevision }) {
      const account = accounts.find((row) => row.id === accountId)
      if (!account) throw notFound()
      if (account.revision !== expectedRevision) throw new MailxClientError({ code: 'conflict', message: '', field: null })
      return oauthSessions.begin({ kind: 'reauthorize', providerId, accountId })
    },
    async finishOAuthAccount(sessionId) {
      const session = await oauthSessions.wait(sessionId, 'create')
      if (session.hint.startsWith('denied')) {
        oauthSessions.cancel(sessionId)
        throw new MailxClientError({ code: 'oauthAuthorizationDenied', message: '', field: null })
      }
      // Like the host: the sign-in's address is tried first, and a refused or
      // missing one leaves the address to the user.
      if (session.hint.startsWith('alias') || session.hint.startsWith('refused')) return { kind: 'needsMailbox', suggestion: session.hint }
      session.mailbox = session.hint
      return { kind: 'verified', emailAddress: session.hint }
    },
    async confirmOAuthMailbox({ sessionId, emailAddress }) {
      const session = oauthSessions.signedIn(sessionId)
      if (session.mailbox !== null) throw new MailxClientError({ code: 'oauthInvalidResponse', message: '', field: null })
      const typed = emailAddress.trim()
      session.attempts += 1
      if (!/^[^\s@]+@[^\s@]+$/.test(typed) || typed.toLowerCase().startsWith('refused')) {
        if (session.attempts >= MOCK_MAILBOX_ATTEMPTS) oauthSessions.cancel(sessionId)
        throw new MailxClientError({ code: 'oauthMailboxRefused', message: '', field: null })
      }
      session.mailbox = typed
      return { kind: 'verified', emailAddress: typed } satisfies OAuthSignInResult
    },
    async createOAuthAccount({ sessionId, profile }) {
      const session = oauthSessions.signedIn(sessionId)
      if (session.mailbox === null) throw new MailxClientError({ code: 'oauthMailboxRefused', message: '', field: null })
      const displayName = profile.displayName.trim()
      if (!displayName) throw new MailxClientError({ code: 'validation', message: '', field: 'displayName' })
      const account = oauthAccountFromSession(crypto.randomUUID(), session, { ...profile, displayName })
      accounts.push(account)
      folders.push(...buildFolders(account.id, emails, dataset.customFolders))
      oauthSessions.cancel(sessionId)
      return structuredClone(account)
    },
    async finishOAuthReauthorization(sessionId) {
      const session = await oauthSessions.wait(sessionId, 'reauthorize')
      const index = accounts.findIndex((row) => row.id === session.accountId)
      if (index < 0) throw notFound()
      const current = accounts[index]
      if (current.authentication.kind !== 'oauth2') throw new MailxClientError({ code: 'unsupported', message: '', field: null })
      accounts[index] = { ...current, revision: current.revision + 1, authentication: { ...current.authentication, state: 'connected' }, updatedAt: new Date().toISOString() }
      oauthSessions.cancel(sessionId)
      return structuredClone(accounts[index])
    },
    async cancelOAuthAccount(sessionId) { oauthSessions.cancel(sessionId) },
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
    async requestPreviews(emailIds) {
      const wanted = emails.filter((email) => emailIds.includes(email.summary.id) && email.summary.previewState === 'missing')
      if (!wanted.length) return
      // Like the host: answer later, and a newer request supersedes one that has not started.
      const request = ++previewRequest
      setTimeout(() => {
        if (request !== previewRequest) return
        const items = wanted.map((email) => {
          email.summary.preview = (email.body?.text ?? '').replace(/\s+/g, ' ').slice(0, 120)
          email.summary.previewState = 'ready'
          return { emailId: email.summary.id, preview: email.summary.preview, previewState: 'ready' as const }
        })
        previewListeners.forEach((listener) => listener({ items }))
      }, 600)
    },
    onEmailPreviews(listener) {
      previewListeners.add(listener)
      return () => { previewListeners.delete(listener) }
    },
    async listEmails(query) {
      const { inScope, items } = matchingEmails(query)
      const offset = query.cursor ? Number(query.cursor) : 0
      const limit = query.limit ?? 50
      const page = items.slice(offset, offset + limit)
      return {
        items: structuredClone(page),
        nextCursor: offset + limit < items.length ? String(offset + limit) : null,
        coverage: {
          headersComplete: true, bodiesComplete: false,
          indexedBodies: inScope.filter((email) => email.summary.bodyState === 'ready').length,
          syncedAfter: null, syncedBefore: null, windowCapped: false,
        },
      }
    },
    async selectEmails(query) {
      // Group rows stand for several messages and cannot be selected.
      const { individual: items } = matchingEmails(query)
      return {
        items: items.slice(0, 500).map(({ id, accountId, folderId, isRead, isStarred, archived, localOnly }) =>
          ({ id, accountId, folderId, isRead, isStarred, archived, localOnly })),
        truncated: items.length > 500,
      }
    },
    async listContactAggregations() {
      return [...contactRules].map(([address, createdAt]) => {
        const own = emails
          .filter((email) => !email.summary.localOnly && email.summary.from.address.toLowerCase() === address)
          .map((email) => email.summary)
          .sort((a, b) => b.receivedAt.localeCompare(a.receivedAt) || b.id.localeCompare(a.id))
        return {
          address,
          name: own[0]?.from.name?.trim() || null,
          messageCount: own.length,
          latestReceivedAt: own[0]?.receivedAt ?? null,
          createdAt,
        }
      }).sort((a, b) => (a.name ?? a.address).toLowerCase().localeCompare((b.name ?? b.address).toLowerCase()) || a.address.localeCompare(b.address))
    },
    async setContactAggregation(address, aggregated) {
      const key = senderAddress(address, 'address')
      if (!aggregated) contactRules.delete(key)
      else if (!contactRules.has(key)) contactRules.set(key, new Date().toISOString())
    },
    async getEmail(emailId) {
      const email = emails.find((row) => row.summary.id === emailId)
      if (!email) throw notFound()
      return structuredClone(email)
    },
    async getConversation(emailId): Promise<ConversationPage> {
      const email = emails.find((row) => row.summary.id === emailId)
      if (!email) throw notFound()
      // The mock has no threading headers: group by subject within the account.
      const key = normalizedSubject(email.summary.subject)
      const related = key
        ? emails.filter((row) => row.summary.accountId === email.summary.accountId && normalizedSubject(row.summary.subject) === key)
        : [email]
      const messages = related
        .map((row) => structuredClone(row.summary))
        .sort((a, b) => a.receivedAt.localeCompare(b.receivedAt) || a.id.localeCompare(b.id))
      return { messages, possiblyIncomplete: false }
    },
    async fetchBody(emailId) {
      const email = emails.find((row) => row.summary.id === emailId)
      if (!email) throw notFound()
      // Like the core: a full body also settles the row's preview.
      if (email.summary.bodyState !== 'ready') {
        email.summary.bodyState = 'ready'
        email.summary.preview = (email.body?.text ?? '').replace(/\s+/g, ' ').slice(0, 120)
        email.summary.previewState = 'ready'
      }
      return structuredClone(email)
    },
    setFlag: async (emailId, flag, wanted) => applyFlag(emailId, flag, wanted),
    async setFlags(emailIds, flag, wanted) {
      const ids = [...new Set(emailIds)]
      if (ids.length === 0 || ids.length > 500) throw new MailxClientError({ code: 'validation', message: '一次只能处理 1 到 500 封邮件。', field: null })
      const results = []
      for (const emailId of ids) {
        const email = emails.find((row) => row.summary.id === emailId)
        // Like the core: a local sent copy reports notFound; one failure leaves the rest alone.
        if (!email || email.summary.localOnly) {
          results.push({ emailId, outcome: 'failed' as const, code: 'notFound' as const })
          continue
        }
        results.push({ emailId, outcome: applyFlag(emailId, flag, wanted), code: null })
      }
      return { results }
    },
    onFlagWriteBack: () => () => undefined,
    async deleteEmails(emailIds, confirmedPermanent, partial = false) {
      const requested = [...new Set(emailIds)]
      if (requested.length === 0) throw new MailxClientError({ code: 'validation', message: '没有要删除的邮件。', field: null })
      const find = (id: string) => emails.find((row) => row.summary.id === id && !row.summary.localOnly)
      const skippedIds = partial ? requested.filter((id) => !find(id)) : []
      if (partial && skippedIds.length === requested.length) throw notFound()
      const picked = requested.filter((id) => !skippedIds.includes(id)).map(find)
      // Like the core: outside Trash a message moves there; in Trash, or in an
      // account without one, it goes for good and needs the user's yes.
      const plans = picked.map((email) => {
        if (!email) throw notFound()
        const source = folders.find((folder) => folder.id === email.summary.folderId)
        const trash = source?.role === 'trash'
          ? undefined
          : folders.find((folder) => folder.accountId === email.summary.accountId && folder.role === 'trash')
        return { email, trash }
      })
      const permanent = plans.some((plan) => !plan.trash)
      if (permanent && !confirmedPermanent) throw new MailxClientError({ code: 'confirmationRequired', message: '永久删除前需要确认。', field: null })
      for (const { email } of plans) emails.splice(emails.indexOf(email), 1)
      const batchId = crypto.randomUUID()
      const timer = setTimeout(() => {
        pendingDeletions.delete(batchId)
        for (const { email, trash } of plans) {
          if (!trash) continue
          email.summary.folderId = trash.id
          emails.push(email)
        }
      }, MOCK_DELETE_UNDO_MS)
      pendingDeletions.set(batchId, { timer, emails: plans.map((plan) => plan.email) })
      return { batchId, undoUntil: new Date(Date.now() + MOCK_DELETE_UNDO_MS).toISOString(), permanent, skippedIds }
    },
    async undoDelete(batchId) {
      const pending = pendingDeletions.get(batchId)
      if (!pending) throw notFound()
      clearTimeout(pending.timer)
      pendingDeletions.delete(batchId)
      emails.push(...pending.emails)
    },
    async archiveEmails(emailIds, partial = false) {
      return changeArchive(emailIds, true, false, partial)
    },
    async unarchiveEmails(emailIds, partial = false) {
      return changeArchive(emailIds, false, false, partial)
    },
    async undoArchive(undoToken) {
      const receipt = archiveReceipts.get(undoToken)
      if (!receipt || receipt.expires <= Date.now()) throw notFound()
      changeArchive(receipt.emailIds, receipt.before, true)
    },
    onMailStateChanged(listener) {
      mailStateListeners.add(listener)
      return () => { mailStateListeners.delete(listener) }
    },
    onDeletions: () => () => undefined,
    refreshSync: async () => undefined,
    getAvatar: async () => null,
    saveAttachment: async () => 'saved' as const,
    clearImageCache: async () => undefined,
    translationStatus: async (emailId) => translationStatusOf(emailId),
    async translateEmail(emailId, targetLanguage) {
      const email = emails.find((row) => row.summary.id === emailId)
      if (!email) throw notFound()
      if (email.summary.localOnly || !email.body?.hasHtml) throw new MailxClientError({ code: 'unsupported', message: '此邮件无法翻译。', field: null })
      const config = settings.settings.translation
      if (!config.provider || !translationSecrets.has(config.provider)) throw translationFailure('translationNotConfigured')
      if (config.consentedProvider !== config.provider) throw translationFailure('translationConsentRequired')
      const target = targetLanguage ?? resolvedTarget()
      // A newer request for the same message supersedes the running one.
      translationRuns.get(emailId)?.cancel()
      const translated = await new Promise<true>((resolve, reject) => {
        const timer = setTimeout(() => { translationRuns.delete(emailId); resolve(true) }, MOCK_TRANSLATION_MS)
        translationRuns.set(emailId, { cancel: () => { clearTimeout(timer); translationRuns.delete(emailId); reject(translationFailure('translationCancelled')) } })
      })
      if (translated) {
        const source = detectLanguage(email.body?.text ?? '')
        const seed = dataset.seeds[emails.indexOf(email)]
        translations.set(translationKey(emailId, target), {
          provider: config.provider, sourceLanguage: source,
          subject: seed?.translatedSubject ?? `[${target}] ${email.summary.subject}`,
        })
      }
      return translationStatusOf(emailId, target)
    },
    cancelTranslation: async (emailId) => { translationRuns.get(emailId)?.cancel() },
    clearTranslation: async (emailId) => {
      for (const key of [...translations.keys()]) if (key.startsWith(`${emailId}|`)) translations.delete(key)
    },
    clearTranslations: async () => { translations.clear() },
    translationCacheStats: async () => {
      const count = new Set([...translations.keys()].map((key) => key.split('|')[0])).size
      return { count, bytes: count * 41_000 }
    },
    translationKeys: async () => MOCK_TRANSLATION_KEYS.map((provider) => ({ provider, configured: translationSecrets.has(provider) })),
    setTranslationKey: async (provider, key) => {
      if (!MOCK_TRANSLATION_KEYS.includes(provider) || !key.trim()) throw new MailxClientError({ code: 'validation', message: '配置参数无效。', field: 'key' })
      translationSecrets.set(provider, key.trim())
      return MOCK_TRANSLATION_KEYS.map((id) => ({ provider: id, configured: translationSecrets.has(id) }))
    },
    removeTranslationKey: async (provider) => {
      translationSecrets.delete(provider)
      return MOCK_TRANSLATION_KEYS.map((id) => ({ provider: id, configured: translationSecrets.has(id) }))
    },
    translationProxyPassword: async () => ({ configured: proxyPassword !== null }),
    setTranslationProxyPassword: async (password) => {
      // Not trimmed: a password may start or end with a space.
      if (!password || /\p{Cc}/u.test(password)) throw new MailxClientError({ code: 'validation', message: '配置参数无效。', field: 'translation.route.password' })
      proxyPassword = password
      return { configured: true }
    },
    removeTranslationProxyPassword: async () => {
      proxyPassword = null
      return { configured: false }
    },
    // Browser preview only: a key containing "bad" is refused, so the failure text can be seen.
    testTranslation: async () => {
      await new Promise((resolve) => setTimeout(resolve, 500))
      const { provider, route } = settings.settings.translation
      const key = provider ? translationSecrets.get(provider) : undefined
      if (!provider || key === undefined) throw translationFailure('translationNotConfigured')
      if (key.includes('bad')) throw translationFailure('translationKeyRejected')
      if (route.kind === 'proxy' && (route.host.includes('bad') || (route.username !== '' && proxyPassword?.includes('bad')))) throw translationFailure('translationProxy')
      return { translated: resolvedTarget().startsWith('zh') ? '你好，这是一条测试。' : 'Hello, this is a test.' }
    },
    prepareBody: async (emailId, _images, translation) => {
      if (translation !== undefined && !translations.has(translationKey(emailId, translation))) throw notFound()
      return { documentToken: crypto.randomUUID() }
    },
    prepareDraftBody: async (draftId, expectedRevision) => {
      const draft = requireDraft(draftId)
      if (draft.revision !== expectedRevision) throw new MailxClientError({ code: 'conflict', message: '草稿已被修改，请重新加载后重试。', field: null })
      return { documentToken: crypto.randomUUID() }
    },
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
      const draft = newDraft({ accountId, kind: 'new', origin: 'user', sourceEmailId: null, to: request.to, cc: request.cc, bcc: request.bcc, subject: request.subject, ...mockDraftBody(request.bodyHtml), inReplyTo: null, references: null })
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
        accountId: email.summary.accountId, kind: request.kind, origin: 'user', sourceEmailId: email.summary.id,
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
    async addInlineImages(draftId, expectedRevision) {
      const draft = mutableDraft(draftId, expectedRevision)
      const inserted = [addMockInline(draft, 'image.png', MOCK_PIXEL)]
      return { draft: touchDraft(draft), inserted }
    },
    async addInlineImageBytes(draftId, expectedRevision, fileName, bytes) {
      const draft = mutableDraft(draftId, expectedRevision)
      const inserted = [addMockInline(draft, fileName || 'image.png', bytes)]
      return { draft: touchDraft(draft), inserted }
    },
    async inlineImagePreview(_draftId, attachmentId) {
      const bytes = inlineBytes.get(attachmentId)
      if (!bytes) throw notFound()
      return bytes.slice().buffer
    },
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
      const sender = accounts.find((account) => account.id === draft.accountId)
      if (!sender) throw notFound()
      lockedDrafts.add(draftId)
      announceDraft({ draftId, accountId: draft.accountId, change: 'locked' })
      return clone(outbox.queue({
        clientMutationId, draftId, accountId: draft.accountId, senderEmailAddress: sender.emailAddress, subject: draft.subject,
        recipients: recipients.slice(0, 3).map((r) => r.address), recipientCount: recipients.length,
        undoMs: settings.settings.compose.undoSendSeconds * 1000,
        simulate: simulatedOutcome(recipients.map((r) => r.address)),
      }))
    },
    listOutbox: async ({ accountId = null, includeAll = false } = {}) => clone(outbox.list(accountId, includeAll)),
    async getOutbox(id) { return clone(outbox.require(id)) },
    async cancelOutbox(id) { return clone(outbox.cancel(id)) },
    async removeLocalCopy(emailId) {
      const index = emails.findIndex((row) => row.summary.id === emailId)
      if (index < 0) throw notFound()
      if (!emails[index].summary.localOnly) throw new MailxClientError({ code: 'validation', message: '只能移除仅在本机的副本。', field: null })
      const [{ summary }] = emails.splice(index, 1)
      sentCopiesListeners.forEach((listener) => listener({ accountId: summary.accountId }))
    },
    async retryOutboxNow(id) { return clone(outbox.retryNow(id)) },
    async resolveUnknown(request) { return clone(outbox.resolve(request.id, request.resolution)) },
    async openComposer(draftId, fresh = false) {
      const draft = clone(requireDraft(draftId))
      options.onOpenComposer?.(draft, fresh)
    },
    closeComposer: async () => undefined,
    cancelComposerClose: async () => undefined,
    onOutbox: (listener) => outbox.subscribe(listener),
    onSentCopiesChanged(listener) { sentCopiesListeners.add(listener); return () => { sentCopiesListeners.delete(listener) } },
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
    openExternalLink: async (url) => { window.open(url, '_blank', 'noopener,noreferrer') },
    onUpdateState: () => () => undefined,
    // The browser preview has no host: the switch only changes an in-memory status.
    getMcpStatus: async () => structuredClone(mcpStatus),
    getAutostart: async () => ({ enabled: false, available: false }),
    setAutostart: async () => { throw new MailxClientError({ code: 'hostUnavailable', message: '', field: null }) },
    async setMcpEnabled(enabled) {
      mcpStatus = enabled ? { state: 'running', clients: [] } : { state: 'off' }
      for (const listener of mcpListeners) listener(structuredClone(mcpStatus))
      return structuredClone(mcpStatus)
    },
    getMcpClientConfig: async (): Promise<McpClientConfig> => ({ command: '/Applications/MailX.app/Contents/MacOS/mailx-mcp', args: ['--discovery', '/Users/mock/Library/Application Support/MailX/mcp-discovery.json'], display: { command: '/Applications/MailX.app/Contents/MacOS/mailx-mcp', discovery: '~/Library/Application Support/MailX/mcp-discovery.json' } }),
    onMcpStatus(listener) { mcpListeners.add(listener); return () => { mcpListeners.delete(listener) } },
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
      if (next.translation?.alwaysTranslate.some((code) => next.translation?.neverSuggest.includes(code))) {
        throw new MailxClientError({ code: 'validation', field: 'translation.alwaysTranslate', message: '配置参数无效。' })
      }
      const appearance = next.appearance ?? settings.settings.appearance
      const interfaceFont = parseFontFamilies(appearance.interfaceFont).join(', ')
      const mailFont = parseFontFamilies(appearance.mailFont).join(', ')
      const general = next.general ?? settings.settings.general
      const layout = { ...settings.settings.layout, ...next.layout }
      const order = normalizedAccountOrder(layout.accountOrder)
      if (!order) throw new MailxClientError({ code: 'validation', field: 'layout.accountOrder', message: '配置参数无效。' })
      layout.accountOrder = order
      settings = { revision: settings.revision + 1,
        effectiveLocale: general.language === 'system' ? systemLocale() : general.language,
        settings: { ...settings.settings, ...next, layout, appearance: { ...appearance, interfaceFont, mailFont } }, stored: true, recovered: false }
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

/** A rough guess from the script and common words; the real core detects on the device the same way, only better. */
function detectLanguage(text: string): string | null {
  const sample = text.slice(0, 600)
  if (sample.replace(/\s/g, '').length < 12) return null
  if (/[\u3040-\u30ff]/.test(sample)) return 'ja'
  if (/[\uac00-\ud7af]/.test(sample)) return 'ko'
  if (/[\u4e00-\u9fff]/.test(sample)) return 'zh'
  const words = sample.toLowerCase().match(/[a-zäöüßéèàçñ]+/g) ?? []
  const score = (list: string[]) => words.filter((word) => list.includes(word)).length
  const candidates: [string, number][] = [
    ['de', score(['der', 'die', 'das', 'und', 'ist', 'nicht', 'wir', 'ich', 'für', 'mit'])],
    ['fr', score(['le', 'la', 'les', 'des', 'est', 'nous', 'vous', 'pour', 'avec', 'une'])],
    ['es', score(['el', 'los', 'las', 'una', 'para', 'con', 'que', 'por', 'es', 'nos'])],
    ['en', score(['the', 'and', 'is', 'are', 'to', 'of', 'we', 'you', 'for', 'with', 'this', 'on'])],
  ]
  const best = candidates.sort((a, b) => b[1] - a[1])[0]
  return best[1] >= 2 ? best[0] : null
}

const missing = { code: 'notFound', message: '内容不存在。', field: null } as const

const WORK = '11111111-1111-4111-8111-111111111111'
const PERSONAL = '22222222-2222-4222-8222-222222222222'

const OAUTH_PROVIDER_IDS = ['microsoft-personal', 'microsoft-work']
/** How long the mock "browser sign-in" takes before it comes back. */
const OAUTH_DELAY_MS = 600
/** Typed mailbox addresses a mock session accepts trying, as in the host. */
const MOCK_MAILBOX_ATTEMPTS = 5

/**
 * The address the mock sign-in "returns". The page never asks for one first.
 * To exercise the other paths in the browser preview, set
 * `localStorage['mailx.mock.oauthAddress']`: `alias…` or `refused…` makes the
 * mail server refuse it (the page then asks for an address; `refused…` also
 * fails every typed address starting with `refused`), `denied…` makes the user
 * decline access.
 */
function mockSignInAddress(providerId: string): string {
  const preset = typeof localStorage === 'undefined' ? null : localStorage.getItem('mailx.mock.oauthAddress')
  return (preset?.trim() || (providerId === 'microsoft-personal' ? 'lin@outlook.com' : 'lin@contoso.com')).toLowerCase()
}

type CreateSession = {
  kind: 'create'
  providerId: string
  route: RouteInput
  /** What the sign-in names; tried first. */
  hint: string
  /** The address the mock mail server accepted, once there is one. */
  mailbox: string | null
  attempts: number
}
type OAuthSession = CreateSession | { kind: 'reauthorize'; providerId: string; accountId: string }

/**
 * Stands in for the host's sign-in sessions. Waiting for the browser takes a
 * moment; cancel makes a pending wait fail with `oauthSessionExpired`, as the
 * host does, and drops the session with its "tokens".
 */
function createOAuthSessions() {
  const sessions = new Map<string, { session: OAuthSession; abort: () => void }>()
  const expired = () => new MailxClientError({ code: 'oauthSessionExpired', message: '', field: null })
  return {
    begin(session: OAuthSession): OAuthSessionHandle {
      const sessionId = crypto.randomUUID()
      sessions.set(sessionId, { session, abort: () => undefined })
      return { sessionId }
    },
    wait<K extends OAuthSession['kind']>(sessionId: string, kind: K): Promise<Extract<OAuthSession, { kind: K }>> {
      const entry = sessions.get(sessionId)
      if (!entry || entry.session.kind !== kind) return Promise.reject(expired())
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => resolve(entry.session as Extract<OAuthSession, { kind: K }>), OAUTH_DELAY_MS)
        entry.abort = () => {
          clearTimeout(timer)
          reject(expired())
        }
      })
    },
    /** A session whose browser already returned. */
    signedIn(sessionId: string): CreateSession {
      const entry = sessions.get(sessionId)
      if (!entry || entry.session.kind !== 'create') throw expired()
      return entry.session
    },
    cancel(sessionId: string) {
      const entry = sessions.get(sessionId)
      sessions.delete(sessionId)
      entry?.abort()
    },
  }
}

/** The servers the core's reviewed registrations use; the real host replaces whatever the page sends. */
function registeredServers(providerId: string, emailAddress: string): Pick<AccountConfigInput, 'imap' | 'smtp'> {
  return {
    imap: { host: 'outlook.office365.com', port: 993, tls: 'implicit', username: emailAddress },
    smtp: { host: providerId === 'microsoft-personal' ? 'smtp-mail.outlook.com' : 'smtp.office365.com', port: 587, tls: 'startTls', username: emailAddress },
  }
}

/** Builds the account like the host: address, servers and route from the session, the rest from the profile. */
function oauthAccountFromSession(id: string, session: CreateSession, profile: Pick<AccountConfigInput, 'displayName' | 'color' | 'senderName' | 'useProviderIcon'>): Account {
  const endpoint = (suffix: string) => ({ id: `${id}-${suffix}`, generation: 1, credentialConfigured: true, trust: 'trusted' as const })
  const address = session.mailbox ?? ''
  const { providerId } = session
  const placeholder = { issuer: '', clientRegistrationId: '', authorizationSubject: null, targetMailboxResource: null }
  return {
    ...accountFromConfig(id, 1, {
      ...profile, emailAddress: address, providerId, route: session.route,
      ...registeredServers(providerId, address),
      authentication: { kind: 'oauth2', configuration: { providerId, ...placeholder } },
    }),
    authentication: {
      kind: 'oauth2',
      configuration: {
        providerId,
        issuer: 'https://login.microsoftonline.com/00000000-0000-0000-0000-000000000000/v2.0',
        clientRegistrationId: `${providerId}-v1`,
        authorizationSubject: '00000000-0000-0000-0000-000000000000/00000000-0000-0000-0000-000000000001',
        targetMailboxResource: address,
      },
      state: 'connected',
      imap: endpoint('imap'),
      smtp: endpoint('smtp'),
    },
  }
}

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
      ? { kind: 'password', mode: 'separate', imap: auth('imap'), smtp: auth('smtp') }
      : { kind: 'password', mode: 'shared', imap: auth('shared'), smtp: auth('shared') },
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

function normalizedSubject(subject: string): string {
  let rest = subject.trim()
  for (;;) {
    const next = rest.replace(/^(re|fwd|fw|回复|答复|转发)\s*[:：]\s*/i, '')
    if (next === rest) return rest.trim().toLowerCase()
    rest = next
  }
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
      unreadCount: inFolder.filter((email) => !email.summary.isRead && !email.summary.archived).length,
      totalCount: inFolder.length,
      archivedCount: inFolder.filter((email) => email.summary.archived).length,
      archivedUnreadCount: inFolder.filter((email) => email.summary.archived && !email.summary.isRead).length,
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
  /** A sent copy kept only on this device (`local-sent:` id), like the core's pending server copy. */
  localOnly?: boolean
  /** Recipients of a sent row; defaults to the team address. */
  to?: MailAddress[]
  read?: boolean
  starred?: boolean
  attachments?: [string, string, number][]
  /** The body has an HTML document, so it can be translated. */
  html?: boolean
  /** What the subject reads once translated; defaults to a bracketed copy. */
  translatedSubject?: string
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
    role: 'sent',
    from: ME_WORK,
    to: [{ name: null, address: 'chen.yu@example.com' }, { name: null, address: 'zhao@example.com' }, { name: null, address: 'qian@example.com' }],
    subject: '周五评审安排',
    text: '各位好，周五下午三点评审，会议室已订好，请提前阅读附上的方案。',
    hoursAgo: 0.2,
    read: true,
    localOnly: true,
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
  // A conversation in the work account: two messages from the owner sit among the replies.
  {
    account: WORK,
    from: { name: '周明', address: 'zhou.ming@example.com' },
    subject: '十二月团建地点',
    text: '大家好，十二月团建想去郊外住一晚，目前有两个备选：莫干山和安吉。大家有偏好的话请回复我。\n\n周明',
    hoursAgo: 52,
    read: true,
  },
  {
    account: WORK,
    role: 'sent',
    from: ME_WORK,
    to: [{ name: '周明', address: 'zhou.ming@example.com' }],
    subject: 'Re: 十二月团建地点',
    text: '我倾向安吉，交通更方便，周五晚上出发也来得及。预算大概多少？',
    hoursAgo: 50,
    read: true,
  },
  {
    account: WORK,
    from: { name: '周明', address: 'zhou.ming@example.com' },
    subject: '回复：十二月团建地点',
    text: '人均 800 左右，含住宿和两顿饭。那就先按安吉来，我去问问民宿还有没有空位。',
    hoursAgo: 26,
    read: true,
  },
  {
    account: WORK,
    from: { name: '王芳', address: 'wang.fang@example.com' },
    subject: 'Re: 十二月团建地点',
    text: '好啊，周末可以。我问了两家民宿，12 月 13 日那个周末都有空位，需要的话我先帮大家占房。',
    hoursAgo: 2,
  },
  {
    account: WORK,
    role: 'sent',
    from: ME_WORK,
    to: [{ name: '王芳', address: 'wang.fang@example.com' }],
    subject: 'Re: Re: 十二月团建地点',
    text: '麻烦你先占房，我下午把名单发给你。',
    hoursAgo: 1,
    read: true,
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
  /** Whether some previews start unfetched, as for headers synced but not yet scrolled into view. */
  lazyPreviews: boolean
}

const DEFAULT_DATASET: MockDataset = {
  accounts: DEFAULT_ACCOUNTS,
  seeds: DEFAULT_SEEDS,
  customFolders: DEFAULT_CUSTOM_FOLDERS,
  me: { [WORK]: ME_WORK, [PERSONAL]: ME_HOME },
  htmlFirst: true,
  lazyPreviews: true,
}

/** Fictitious data for README screenshots, selected with `?seed=showcase`; see `mock-showcase.ts`. */
function showcaseDataset(): MockDataset {
  return {
    accounts: SHOWCASE_DATASET.accounts.map(({ id, config }) => accountFromConfig(id, 1, config)),
    seeds: SHOWCASE_DATASET.seeds,
    customFolders: SHOWCASE_DATASET.customFolders,
    me: SHOWCASE_DATASET.me,
    htmlFirst: false,
    lazyPreviews: false,
  }
}

// Relative to load time so the list always looks recent.
const BASE_TIME = Date.now()

function buildEmails({ seeds, me: addresses, htmlFirst, lazyPreviews }: MockDataset): EmailDetail[] {
  return seeds.map((seed, index) => {
    const id = `mock-email-${index + 1}`
    const me = addresses[seed.account]
    const sent = seed.role === 'sent' || seed.role === 'drafts'
    const lazy = lazyPreviews && index % 2 === 1
    const recipients = seed.to ?? (sent ? [{ name: '团队', address: 'team@example.com' }] : [me])
    const summary: EmailSummary = {
      id: seed.localOnly ? `local-sent:${id}` : id,
      accountId: seed.account,
      folderId: seed.folder ? `${seed.account}:${seed.folder}` : folderId(seed.account, seed.role ?? 'inbox'),
      subject: seed.subject,
      from: seed.from,
      to: { addresses: recipients.slice(0, 3), total: recipients.length },
      localOnly: seed.localOnly ?? false,
      archived: false,
      preview: lazy ? '' : seed.text.replace(/\s+/g, ' ').slice(0, 120),
      receivedAt: new Date(BASE_TIME - seed.hoursAgo * 3_600_000).toISOString(),
      isRead: seed.read ?? false,
      isStarred: seed.starred ?? false,
      hasAttachment: (seed.attachments?.length ?? 0) > 0,
      // Lazy rows look like synced headers: no body or preview yet.
      bodyState: lazy ? 'missing' : 'ready',
      previewState: lazy ? 'missing' : 'ready',
      pendingChanges: false,
      matchSnippet: null,
      senderGroup: null,
    }
    return {
      summary,
      to: recipients,
      cc: [],
      replyTo: [],
      body: { text: seed.text, hasHtml: (htmlFirst && index === 0) || seed.html === true, truncated: false, remoteImagesBlocked: htmlFirst && index === 0 ? 1 : 0 },
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

/** Browser preview only (`?seed=outbox`): one rich draft plus sends that end failed, unknown and retrying. */
export async function seedPreviewOutbox(client: MailxClient) {
  const make = async (to: string, subject: string, bodyText: string) => {
    const draft = await client.createDraft({
      accountId: null, to: [{ name: null, address: to }], cc: [], bcc: [], subject, bodyHtml: mockPlainToHtml(bodyText), clientMutationId: crypto.randomUUID(),
    })
    return draft
  }
  const preview = await client.createDraft({
    accountId: null, to: [{ name: null, address: 'team@example.com' }], cc: [], bcc: [], subject: '周会纪要（草稿）', clientMutationId: crypto.randomUUID(),
    bodyHtml: '<h2>本周进展</h2><p>各位好，</p><p>本周已完成 <strong>邮件卡片优化</strong> 和 <span data-color="blue">草稿富文本预览</span>。</p><p><br></p><ul><li><p>状态点与常用操作保持紧凑</p></li><li><p>阅读时保留标题、列表和引用</p></li></ul><p><br></p><blockquote><p>下周重点：<mark data-highlight="yellow">完善阅读体验</mark></p></blockquote><p><br></p><p data-align="right"><em>林伟</em></p>',
  })
  await client.addAttachments(preview.id, preview.revision)
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
  senderEmailAddress: string
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
        id, draftId: input.draftId, accountId: input.accountId, senderEmailAddress: input.senderEmailAddress, messageId: `<mock-${id}@mock.invalid>`, subject: input.subject,
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
        clientMutationId: crypto.randomUUID(), draftId: item.draftId ?? '', accountId: item.accountId, senderEmailAddress: item.senderEmailAddress, subject: item.subject,
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
