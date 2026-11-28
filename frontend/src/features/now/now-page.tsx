import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Alert02Icon, CheckmarkCircle02Icon, MailUpload01Icon, SparklesIcon } from '@hugeicons/core-free-icons'
import { keepPreviousData, useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import type { Account, AccountSyncState, CodeCandidate, Folder, OutboxItem, OverviewCode } from '@/generated/accounts'
import { ErrorNotice } from '@/components/error-notice'
import { toast } from '@/components/ui/toast'
import { cn } from '@/lib/utils'
import { errorDetail } from '@/i18n/errors'
import { latestSettingsSnapshot } from '@/lib/settings-cache'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { useSettings, useUpdateNow } from '@/features/settings/settings-queries'
import { accountIssues, accountProblem } from '@/features/accounts/account-status'
import { signInNeeded } from '@/features/accounts/sign-in-state'
import { emailQuery } from '@/features/mail/format'
import type { Mailbox } from '@/features/mail/mailbox'
import { readyVerificationCode, type VerificationCodes, type VerificationTarget } from '@/features/mail/use-verification-codes'
import { daySegment, lineContext, lineKey, localDay, type LineContext, type NowKey } from './greeting'
import { occasionOn } from './calendar'
import { useNowAmbientMode, useNowAmbientScene } from './ambient-preference'
import { useNowMailActions } from './use-now-mail-actions'
import { nowSuggestions } from './now-suggestions'
import { useDayClock, useFirstVisitToday, useLastVisit, useMinuteClock } from './visit'
import { entranceMode, motionProps } from './motion'
import { useFreshCodes } from './use-fresh-codes'
import { GreetingIcon } from './greeting-icon'
import { NowAmbient } from './now-ambient'
import { relativeTime } from './relative-time'
import { dayStarts, syncSummary } from './activity'
import { SummaryLine } from './summary-line'
import { useSharedAssistantChat } from '@/features/ai/assistant-chat-context'
import { useAssistantUsable } from '@/features/ai/use-assistant-usable'
import { NowChatShell } from './now-chat'
import { useNowSummary } from './use-now-summary'
import { suppressed } from './modules/arrange'
import { OVERVIEW_CODES } from './modules/codes'
import { DEFAULT_NOW } from './modules/defaults'
import { UNREAD_ROWS } from './modules/inbox'
import { ModuleSection } from './modules/module-section'
import { moduleById } from './modules/registry'
import { useSnoozed } from './modules/snooze'
import type { AttentionItem, NowContext, NowData, NowModule, NowModuleId, NowRenderEnv, ShownCode } from './modules/types'
import { useModuleOrder } from './modules/use-module-order'

/** The unified inbox with the Unread filter, newest first, as the workbench asks for it. */
const UNREAD_QUERY = emailQuery({ folderRole: 'inbox', archive: 'exclude', unread: true, limit: UNREAD_ROWS })
/** Compact secondary modules that can pair side-by-side on wide screens when adjacent. */
const PAIRABLE_MODULES = new Set<NowModuleId>(['drafts', 'attachments'])

export type NowPageProps = {
  client: MailxClient
  /** In the user's display order. */
  accounts: Account[]
  folders: Folder[]
  syncStates: Record<string, AccountSyncState>
  draftCount: number
  /** Unfinished sends, as the outbox view lists them. */
  outbox: OutboxItem[]
  onSelectMailbox: (mailbox: Mailbox) => void
  /** The unified inbox with the Unread filter on. */
  onOpenUnread: () => void
  /** One person's mail across folders and accounts, in the workbench's contact view. */
  onOpenContact: (contact: { address: string; name: string | null }) => void
  onOpenAccount: (accountId: string) => void
  /** Shows the message in the reader, in the view it belongs to. */
  onOpenEmail: (email: { id: string; accountId: string }) => void
  /** Opens a draft the assistant wrote, in the compose window. */
  onOpenDraft?: (draftId: string) => void
  /** Changes when something outside (the command palette) asks to start typing to the assistant. */
  focusInputToken?: number
  /** Open on the conversation rather than the overview: the assistant's island just handed it over. */
  openConversation?: boolean
  /** The workbench's shared verification-code controller: recognition and copying behave as in the list and reader. */
  verification: VerificationCodes
}

/**
 * The Now page (now-page.md): a greeting and what is going on in the mailbox,
 * all computed on this device. It fills the island; the content sits in one
 * column, vertically centred while it is shorter than the island and
 * scrolling when taller. Sections appear only with content.
 */
export function NowPage({ client, accounts, folders, syncStates, draftCount, outbox, onSelectMailbox, onOpenUnread, onOpenContact, onOpenAccount, onOpenEmail, onOpenDraft, focusInputToken, openConversation = false, verification }: NowPageProps) {
  const { i18n } = useTranslation()
  const language = i18n.language
  const now = useDayClock()
  const minute = useMinuteClock()
  const since = useLastVisit()
  const firstToday = useFirstVisitToday()
  const reducedMotion = usePrefersReducedMotion()
  // The day's first visit plays the full entrance, later ones a lighter one; reduced motion neither.
  const mode = entranceMode(firstToday, reducedMotion)
  const settings = useSettings(client)
  // The conversation lives above this page (it survives leaving it); the input shows only while the assistant is usable.
  const chat = useSharedAssistantChat()
  const assistant = useAssistantUsable(client)
  const chatOn = chat !== null && assistant.usable
  // Recomputed when the day clock passes midnight; the key changes with it.
  const starts = dayStarts(now)
  const overview = useQuery({
    queryKey: mailKeys.overview(since, starts),
    queryFn: () => client.getMailOverview({ since, dayStarts: starts }),
    placeholderData: keepPreviousData,
  })
  // Under the mail list keys, so read marks, deletions and sync rounds update it like any list.
  const sources = JSON.stringify(accounts.map(({ id, mailboxSourceId }) => [id, mailboxSourceId]).sort())
  const unreadList = useInfiniteQuery({
    queryKey: mailKeys.emails(sources, UNREAD_QUERY),
    queryFn: () => client.listEmails(UNREAD_QUERY),
    initialPageParam: null as string | null,
    getNextPageParam: () => null,
    placeholderData: keepPreviousData,
  })
  useInboxRefresh(folders, syncStates, unreadList.refetch)

  // Nothing but the backdrop until both answers are in (or failed), so the
  // greeting never moves once shown; after that it stays.
  const [shown, setShown] = useState(false)
  if (!shown && !overview.isPending && !unreadList.isPending) setShown(true)

  const data = overview.data
  // The AI sentence starts once the overview is in, for the same period the page shows.
  const summary = useNowSummary({ client, ready: data !== undefined, since, dayStarts: starts })
  const freshCodes = useFreshCodes(data?.verificationCodes ?? [])
  const freshCandidates = useFreshCodes(data?.codeCandidates ?? [])
  useCodeTargets(verification, [...freshCodes, ...freshCandidates])
  // Only codes the shared recognizer has confirmed, so every one shown can be copied. A candidate
  // (a likely verification mail the overview found no code in yet) counts once the recognizer says so.
  const codes = [...freshCodes, ...freshCandidates].flatMap((code): ShownCode[] => {
    const ready = readyVerificationCode(verification.status(code.emailId))
    return ready ? [{ emailId: code.emailId, accountId: code.accountId, from: code.from, receivedAt: code.receivedAt, ready }] : []
  }).sort((a, b) => Date.parse(b.receivedAt) - Date.parse(a.receivedAt)).slice(0, OVERVIEW_CODES)

  // Needs attention: accounts that stopped, sends that failed or may not have gone. Drafts are a module of their own.
  const attention: AttentionItem[] = accounts.flatMap((account): AttentionItem[] => {
    const sync = syncStates[account.id]
    if (accountProblem(account, sync) !== 'stopped') return []
    const issues = accountIssues(account, sync)
    const key = issues.needsTrust ? 'now:accountConfirm'
      : signInNeeded(account, issues.stoppedForSignIn) ? 'now:accountSignIn' : 'now:accountStopped'
    return [{ id: `account:${account.id}`, icon: Alert02Icon, tone: 'alert', avatar: account, label: t(key, { account: account.displayName }), onClick: () => onOpenAccount(account.id) }]
  })
  const failed = outbox.filter((item) => item.state === 'failed').length
  const unknown = outbox.filter((item) => item.state === 'unknown').length
  if (failed > 0) attention.push({ id: 'failed', icon: MailUpload01Icon, tone: 'alert', label: t('now:outboxFailed', { count: failed }), onClick: () => onSelectMailbox({ kind: 'outbox' }) })
  if (unknown > 0) attention.push({ id: 'unknown', icon: MailUpload01Icon, tone: 'alert', label: t('now:outboxUnknown', { count: unknown }), onClick: () => onSelectMailbox({ kind: 'outbox' }) })
  // Drafts never set the "attention" line: they pile up and never clear themselves.
  const urgent = attention.length > 0

  // Unread in the unified inbox, as the sidebar counts it; unknown counts show nothing.
  const inboxes = folders.filter((folder) => folder.selectable && folder.role === 'inbox')
  const unread = inboxes.every((folder) => folder.countsKnown) ? inboxes.reduce((total, folder) => total + folder.unreadCount, 0) : 0
  const accountUnread = accounts.length >= 2
    ? accounts.flatMap((account) => {
      const folder = inboxes.find((item) => item.accountId === account.id)
      return folder && folder.countsKnown ? [{ account, folder, unread: folder.unreadCount }] : []
    })
    : []
  const unreadRows = (unreadList.data?.pages[0]?.items ?? []).slice(0, UNREAD_ROWS)
  const starredUnread = data?.starredUnread ?? 0
  const newMail = data?.newMail.total ?? 0
  const attachments = data?.recentAttachments ?? []
  const moduleData: NowData = {
    attention,
    draftCount,
    codes,
    inbox: { newMail, unread, starredUnread, rows: unreadRows, accountUnread },
    attachments,
    activity: data?.activity,
    away: { sinceTime: Date.parse(since), ms: now.getTime() - Date.parse(since), total: newMail, senders: data?.newMail.senders ?? [] },
    unreadSenders: data?.unreadSenders ?? [],
  }
  const sync = syncSummary(accounts, syncStates)

  const segment = daySegment(now)
  const occasion = occasionOn(now, language)
  const [ambientMode] = useNowAmbientMode()
  const [ambientScene] = useNowAmbientScene()
  const [inConversation, setInConversation] = useState(openConversation)
  const mailActions = useNowMailActions(client)
  const context = lineContext(now, language, { attention: urgent, newMail, unread })
  const nothingPending = data !== undefined && attention.length === 0 && draftCount === 0 && newMail === 0 && unread === 0
    && starredUnread === 0 && codes.length === 0 && attachments.length === 0

  const defaultId = settings.data?.settings.compose.defaultAccountId
  const name = (accounts.find((account) => account.id === defaultId) ?? accounts[0])?.senderName?.trim() || null
  const greeting = name ? t(`now:greetingNamed.${segment}`, { name }) : t(`now:greeting.${segment}`)
  const date = new Intl.DateTimeFormat(language, { month: 'long', day: 'numeric', weekday: 'long' }).format(now)

  // The order is worked out once for the stay; hiding and snoozing apply as they happen.
  const nowSettings = settings.data?.settings.now ?? DEFAULT_NOW
  const sinceTime = Date.parse(since)
  const slots = useModuleOrder(shown && !settings.isPending, nowSettings, { now, awayMs: now.getTime() - sinceTime, firstToday, data: moduleData } satisfies NowContext)
  const { snoozed, snooze, restore } = useSnoozed()
  const today = localDay(now)
  const updateNow = useUpdateNow(client)
  const queryClient = useQueryClient()

  const hideForever = (module: NowModule) => {
    const current = latestSettingsSnapshot(queryClient)?.settings.now ?? nowSettings
    if (current.hidden.includes(module.id)) return
    const failure = () => toast.error(t('now:hideFailed'))
    updateNow.mutate({ hidden: [...current.hidden, module.id] }, { onError: failure })
    toast.info(t('now:hidden', { module: module.title() }), {
      description: t('now:hiddenHint'),
      action: {
        label: t('now:undo'),
        onClick: () => {
          const latest = latestSettingsSnapshot(queryClient)?.settings.now ?? current
          updateNow.mutate({ hidden: latest.hidden.filter((id) => id !== module.id) }, { onError: failure })
        },
      },
    })
  }
  const hideToday = (module: NowModule) => {
    snooze(module.id, module.signature(moduleData))
    // The module's button is gone with it; the toast offers the way back.
    toast.info(t('now:hiddenToday', { module: module.title() }), {
      action: { label: t('now:undo'), onClick: () => restore(module.id) },
    })
  }
  const env: NowRenderEnv = {
    data: moduleData,
    language,
    minute,
    mode,
    sinceTime,
    verification,
    contactAccountId: accounts[0]?.id ?? '',
    client,
    folders,
    mailActions,
    actions: { onSelectMailbox, onOpenUnread, onOpenContact, onOpenAccount, onOpenEmail },
  }
  // A failed overview is reported once, where its first module would be.
  const overviewFailed = overview.isError && data === undefined
  let overviewReported = false
  let index = 1
  const next = () => index++
  const visibleModules: NowModule[] = []
  const errorAt = new Set<NowModuleId>()
  for (const id of slots ?? ([] as NowModuleId[])) {
    const module = moduleById(id)
    if (suppressed(module, nowSettings, snoozed, moduleData, today)) continue
    if (overviewFailed && module.fromOverview && !overviewReported) {
      overviewReported = true
      errorAt.add(module.id)
    }
    if (module.hasContent(moduleData)) visibleModules.push(module)
  }
  // Adjacent compact modules pair into two columns on wide screens without changing order.
  const paired = new Set<NowModuleId>()
  for (let i = 0; i < visibleModules.length - 1; i += 1) {
    const current = visibleModules[i]
    const following = visibleModules[i + 1]
    if (PAIRABLE_MODULES.has(current.id) && PAIRABLE_MODULES.has(following.id)) {
      paired.add(current.id)
      paired.add(following.id)
      i += 1
    }
  }
  const blocks: ReactNode[] = []
  let errorRendered = false
  for (const module of visibleModules) {
    if (errorAt.has(module.id)) {
      errorRendered = true
      blocks.push(
        <div key="overview-error" {...motionProps(mode, next(), 'px-3 lg:col-span-2')}>
          <ErrorNotice message={t('now:overviewFailed')} detail={errorDetail(overview.error)} actionLabel={t('now:retry')} onAction={() => void overview.refetch()} actionDisabled={overview.isFetching} />
        </div>,
      )
    }
    blocks.push(
      <ModuleSection
        key={module.id}
        title={module.title()}
        compact={module.compact}
        mode={mode}
        index={next()}
        className={paired.has(module.id) ? 'lg:col-span-1' : 'lg:col-span-2'}
        menu={module.hideable ? { onHideToday: () => hideToday(module), onHideForever: () => hideForever(module) } : undefined}
      >
        {module.render(env)}
      </ModuleSection>,
    )
  }
  if (overviewReported && !errorRendered) {
    blocks.push(
      <div key="overview-error" {...motionProps(mode, next(), 'px-3 lg:col-span-2')}>
        <ErrorNotice message={t('now:overviewFailed')} detail={errorDetail(overview.error)} actionLabel={t('now:retry')} onAction={() => void overview.refetch()} actionDisabled={overview.isFetching} />
      </div>,
    )
  }

  // The conversation view keeps this much of the overview: the greeting and, when known, the unread count.
  const chatSummary = unread > 0 ? `${greeting} · ${t('ai:chat.summaryUnread', { count: unread })}` : greeting
  const suggestions = nowSuggestions(moduleData)

  return (
    <div data-slot="now-page" className="relative min-h-0 flex-1 overflow-hidden bg-background">
      <NowAmbient segment={segment} scene={ambientScene} occasion={occasion} mode={ambientMode} dimmed={inConversation} />
      <NowChatShell
        client={client}
        chat={chat}
        usable={assistant.usable}
        service={assistant.service}
        summary={chatSummary}
        suggestions={suggestions}
        onConversationChange={setInConversation}
        onOpenEmail={onOpenEmail}
        onOpenDraft={onOpenDraft ?? noop}
        accounts={accounts}
        folders={folders}
        onSelectMailbox={onSelectMailbox}
        focusToken={focusInputToken}
        openConversation={openConversation}
      >
      <div className="relative h-full overflow-y-auto">
        {/* The column sits between two spacers that share the free height 2:3, so a
            short page rests a little above the middle and a long one scrolls. */}
        <div className="flex min-h-full flex-col px-5 sm:px-10">
          <div aria-hidden className="min-h-10 flex-[2] sm:min-h-14" />
          {shown && (
            <div data-slot="now-content" className="mx-auto grid w-full max-w-210 shrink-0 grid-cols-1 gap-x-6 gap-y-8 lg:grid-cols-2">
              <header {...motionProps(mode, 0, 'px-3 lg:col-span-2')}>
                <p className="text-sm text-muted-foreground">{date}</p>
                <h1 className="mt-1 text-3xl font-semibold tracking-tight text-balance"><GreetingIcon segment={segment} mode={mode} />{greeting}</h1>
                <SummaryLine client={client} preset={data ? summaryLine(now, context) : null} summary={summary} mode={mode} />
                {nothingPending && (
                  <div data-slot="now-quiet-badge" className="mt-4 inline-flex items-center gap-2 rounded-full bg-muted/55 px-3.5 py-1.5 text-sm text-muted-foreground">
                    <HugeiconsIcon aria-hidden icon={SparklesIcon} className="size-3.5 shrink-0 text-primary/70" />
                    <span>{context.kind !== 'clear' ? t('now:allClear') : t('now:quietMoment')}</span>
                  </div>
                )}
              </header>

              {blocks}

              {sync && (
                <p data-slot="now-sync" {...motionProps(mode, next(), 'flex items-center gap-1.5 px-3 text-xs text-muted-foreground lg:col-span-2')}>
                  {sync.kind === 'synced' ? (
                    <>
                      {t('now:syncedAt', { time: relativeTime(sync.at, language, minute) })}
                      <HugeiconsIcon
                        aria-hidden
                        data-slot="now-sync-icon"
                        icon={CheckmarkCircle02Icon}
                        className="size-3.5 shrink-0 text-[color-mix(in_oklab,var(--color-emerald-600)_60%,var(--muted-foreground))] dark:text-[color-mix(in_oklab,var(--color-emerald-400)_60%,var(--muted-foreground))]"
                      />
                    </>
                  ) : t(sync.kind === 'syncing' ? 'now:syncing' : 'now:syncIssue')}
                </p>
              )}
            </div>
          )}
          {/* Room for the input when it is there, so the last block can scroll clear of it. */}
          <div aria-hidden className={cn('flex-[3]', chatOn ? 'min-h-[calc(var(--chat-footer-h,0px)+2rem)]' : 'min-h-16')} />
        </div>
      </div>
      </NowChatShell>
    </div>
  )
}

const noop = () => undefined

/**
 * Refetches the unread list when a sync round changed an inbox: the workbench
 * only marks other lists stale, and this one is on screen.
 */
function useInboxRefresh(folders: Folder[], syncStates: Record<string, AccountSyncState>, refetch: () => unknown) {
  const revision = JSON.stringify(folders.filter((folder) => folder.selectable && folder.role === 'inbox').map((folder) => [
    folder.id,
    syncStates[folder.accountId]?.generation ?? 0,
    syncStates[folder.accountId]?.folders.find((state) => state.folderId === folder.id)?.dataRevision ?? 0,
  ]).sort())
  const seen = useRef(revision)
  useEffect(() => {
    if (seen.current === revision) return
    seen.current = revision
    void refetch()
  }, [revision, refetch])
}

/** Asks the shared controller to recognize the codes and candidates; nothing while the page is gone. */
function useCodeTargets(verification: VerificationCodes, codes: (OverviewCode | CodeCandidate)[]) {
  const key = JSON.stringify(codes.map((code): VerificationTarget => ({ emailId: code.emailId, bodyState: code.bodyState })))
  const targets = useMemo(() => JSON.parse(key) as VerificationTarget[], [key])
  const request = verification.requestVisible
  useEffect(() => { request(targets) }, [request, targets])
  useEffect(() => () => { request([]) }, [request])
}

function summaryLine(now: Date, context: LineContext) {
  const key: NowKey = lineKey(now, context)
  switch (context.kind) {
    case 'newMail': return t(key, { count: context.count })
    case 'holiday': return t(key, { holiday: t(`now:holidays.${context.holiday}`) })
    case 'solarTerm': return t(key, { term: t(`now:solarTerms.${context.term}`) })
    default: return t(key)
  }
}
