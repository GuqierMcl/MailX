import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import { Alert02Icon, Attachment01Icon, CheckmarkCircle02Icon, Copy01Icon, File01Icon, MailUpload01Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import { keepPreviousData, useInfiniteQuery, useQuery } from '@tanstack/react-query'
import type { Account, AccountSyncState, CodeCandidate, EmailSummary, Folder, OutboxItem, OverviewActivity, OverviewCode, OverviewContact } from '@/generated/accounts'
import { IconButton } from '@/components/icon-button'
import { cn } from '@/lib/utils'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { useSettings } from '@/features/settings/settings-queries'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { accountIssues, accountProblem } from '@/features/accounts/account-status'
import { signInNeeded } from '@/features/accounts/sign-in-state'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { displayName, emailQuery } from '@/features/mail/format'
import type { Mailbox } from '@/features/mail/mailbox'
import { readyVerificationCode, type ReadyVerificationCode, type VerificationCodes, type VerificationTarget } from '@/features/mail/use-verification-codes'
import { daySegment, lineContext, lineKey, type LineContext, type NowKey } from './greeting'
import { useDayClock, useFirstVisitToday, useLastVisit, useMinuteClock } from './visit'
import { entrance, entranceMode, useCountUp, type EntranceMode } from './motion'
import { useFreshCodes } from './use-fresh-codes'
import { GreetingIcon } from './greeting-icon'
import { NowAmbient } from './now-ambient'
import { relativeTime } from './relative-time'
import { compareWeeks, dayStarts, syncSummary } from './activity'
import { ActivityChart } from './activity-chart'
import { SummaryLine } from './summary-line'
import { useNowSummary } from './use-now-summary'

/** Rows of unread inbox mail the page lists; more are a link away. */
export const UNREAD_ROWS = 6

/** The unified inbox with the Unread filter, newest first, as the workbench asks for it. */
const UNREAD_QUERY = emailQuery({ folderRole: 'inbox', archive: 'exclude', unread: true, limit: UNREAD_ROWS })

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
  onOpenContact: (contact: OverviewContact) => void
  onOpenAccount: (accountId: string) => void
  /** Shows the message in the reader, in the view it belongs to. */
  onOpenEmail: (email: { id: string; accountId: string }) => void
  /** The workbench's shared verification-code controller: recognition and copying behave as in the list and reader. */
  verification: VerificationCodes
}

type AttentionItem = { id: string; icon: IconSvgElement; label: string; tone?: 'alert'; onClick: () => void; avatar?: Account }

/**
 * The Now page (now-page.md): a greeting and what is going on in the mailbox,
 * all computed on this device. It fills the island; the content sits in one
 * column, vertically centred while it is shorter than the island and
 * scrolling when taller. Sections appear only with content.
 */
export function NowPage({ client, accounts, folders, syncStates, draftCount, outbox, onSelectMailbox, onOpenUnread, onOpenContact, onOpenAccount, onOpenEmail, verification }: NowPageProps) {
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

  // Needs attention: accounts that stopped, sends that failed or may not have gone, drafts.
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
  // Drafts are listed but never set the "attention" line: they pile up and never clear themselves.
  const urgent = attention.length > 0
  if (draftCount > 0) attention.push({ id: 'drafts', icon: File01Icon, label: t('now:drafts', { count: draftCount }), onClick: () => onSelectMailbox({ kind: 'drafts' }) })

  // Unread in the unified inbox, as the sidebar counts it; unknown counts show nothing.
  const inboxes = folders.filter((folder) => folder.selectable && folder.role === 'inbox')
  const unread = inboxes.every((folder) => folder.countsKnown) ? inboxes.reduce((total, folder) => total + folder.unreadCount, 0) : 0
  const unreadRows = (unreadList.data?.pages[0]?.items ?? []).slice(0, UNREAD_ROWS)
  const starredUnread = data?.starredUnread ?? 0
  const newMail = data?.newMail.total ?? 0
  const attachments = data?.recentAttachments ?? []
  const activity = data?.activity
  const showInbox = newMail > 0 || unread > 0 || starredUnread > 0 || unreadRows.length > 0
  const showActivity = activity !== undefined
    && (activity.days.some((day) => day.received > 0) || activity.sentLastWeek + activity.sentPreviousWeek > 0)
  const sync = syncSummary(accounts, syncStates)

  const segment = daySegment(now)
  const context = lineContext(now, language, { attention: urgent, newMail, unread })
  const nothingPending = data !== undefined && attention.length === 0 && newMail === 0 && unread === 0
    && starredUnread === 0 && codes.length === 0 && attachments.length === 0

  const defaultId = settings.data?.settings.compose.defaultAccountId
  const name = (accounts.find((account) => account.id === defaultId) ?? accounts[0])?.senderName?.trim() || null
  const greeting = name ? t(`now:greetingNamed.${segment}`, { name }) : t(`now:greeting.${segment}`)
  const date = new Intl.DateTimeFormat(language, { month: 'long', day: 'numeric', weekday: 'long' }).format(now)
  const sinceTime = Date.parse(since)

  let index = 1
  const next = () => index++

  return (
    <div data-slot="now-page" className="relative min-h-0 flex-1 overflow-hidden bg-background">
      <NowAmbient segment={segment} />
      <div className="relative h-full overflow-y-auto">
        {/* The column sits between two spacers that share the free height 2:3, so a
            short page rests a little above the middle and a long one scrolls. */}
        <div className="flex min-h-full flex-col px-5 sm:px-10">
          <div aria-hidden className="min-h-10 flex-[2] sm:min-h-14" />
          {shown && (
            <div data-slot="now-content" className="mx-auto flex w-full max-w-190 shrink-0 flex-col gap-9">
              <header {...motionProps(mode, 0, 'px-3')}>
                <p className="text-sm text-muted-foreground">{date}</p>
                <h1 className="mt-1 text-3xl font-semibold tracking-tight text-balance"><GreetingIcon segment={segment} mode={mode} />{greeting}</h1>
                <SummaryLine client={client} preset={data ? summaryLine(now, context) : null} summary={summary} mode={mode} />
                {nothingPending && context.kind !== 'clear' && <p className="mt-1 text-base text-muted-foreground">{t('now:allClear')}</p>}
              </header>

              {attention.length > 0 && (
                <Section title={t('now:attention')} mode={mode} index={next()}>
                  <ul className="flex flex-col rounded-xl bg-muted/40 p-1">
                    {attention.map((item) => (
                      <li key={item.id}>
                        <button type="button" onClick={item.onClick} className={rowClass}>
                          {item.avatar ? (
                            <AccountAvatar displayName={item.avatar.displayName} color={item.avatar.color} providerId={item.avatar.providerId} useProviderIcon={item.avatar.useProviderIcon} className="size-5 text-[10px]" />
                          ) : <HugeiconsIcon icon={item.icon} className={cn('size-4 shrink-0 text-muted-foreground', item.tone === 'alert' && 'text-destructive')} />}
                          <span className="min-w-0 flex-1 truncate">{item.label}</span>
                          {item.avatar && <HugeiconsIcon icon={Alert02Icon} className="size-4 shrink-0 text-destructive" />}
                        </button>
                      </li>
                    ))}
                  </ul>
                </Section>
              )}

              {codes.length > 0 && (
                <Section title={t('now:codes')} mode={mode} index={next()}>
                  <ul className="flex flex-col rounded-xl bg-muted/40 p-1">
                    {codes.map((code) => (
                      <CodeRow key={code.emailId} code={code} language={language} now={minute} verification={verification} onOpenEmail={onOpenEmail} />
                    ))}
                  </ul>
                </Section>
              )}

              {showInbox && (
                <Section title={t('now:inbox')} mode={mode} index={next()}>
                  <div className="flex flex-wrap gap-2">
                    {newMail > 0 && <StatButton value={newMail} label={t('now:statNew')} mode={mode} onClick={() => onSelectMailbox({ kind: 'unified', role: 'inbox' })} />}
                    <StatButton value={unread} label={t('now:statUnread')} mode={mode} onClick={onOpenUnread} />
                    {starredUnread > 0 && <StatButton value={starredUnread} label={t('now:starredUnread')} mode={mode} onClick={() => onSelectMailbox({ kind: 'starred' })} />}
                  </div>
                  {unreadRows.length > 0 && (
                    <ul className="flex flex-col">
                      {unreadRows.map((email) => (
                        <li key={email.id}>
                          <MailRow email={email} language={language} now={minute} isNew={Date.parse(email.receivedAt) > sinceTime} onOpen={onOpenEmail} />
                        </li>
                      ))}
                    </ul>
                  )}
                  {unread > UNREAD_ROWS && (
                    <button type="button" onClick={onOpenUnread} className={cn(linkClass, 'self-start')}>
                      {t('now:viewAllUnread', { count: unread })}
                    </button>
                  )}
                </Section>
              )}

              {showActivity && (
                <Section title={t('now:recent')} mode={mode} index={next()}>
                  <Activity activity={activity} language={language} mode={mode} contactAccountId={accounts[0]?.id ?? ''} onOpenContact={onOpenContact} />
                </Section>
              )}

              {attachments.length > 0 && (
                <Section title={t('now:attachments')} mode={mode} index={next()}>
                  <ul className="flex flex-col">
                    {attachments.map(({ email, fileNames }) => (
                      <li key={email.id}>
                        <button type="button" onClick={() => onOpenEmail(email)} className={rowClass}>
                          <HugeiconsIcon icon={Attachment01Icon} className="size-4 shrink-0 text-muted-foreground" />
                          <span className="min-w-0 flex-1">
                            <span className="block truncate">
                              {fileNames[0] ?? t('now:attachment')}
                              {fileNames.length > 1 && <span className="text-muted-foreground"> {t('now:moreFiles', { count: fileNames.length - 1 })}</span>}
                            </span>
                            <span className="block truncate text-xs text-muted-foreground">{displayName(email.from)}</span>
                          </span>
                          <span className="shrink-0 text-xs text-muted-foreground">{relativeTime(email.receivedAt, language, minute)}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </Section>
              )}

              {sync && (
                <p data-slot="now-sync" {...motionProps(mode, next(), 'flex items-center gap-1.5 px-3 text-xs text-muted-foreground')}>
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
          <div aria-hidden className="min-h-16 flex-[3]" />
        </div>
      </div>
    </div>
  )
}

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

/** The last two weeks: the daily chart, then a few plain sentences. */
function Activity({ activity, language, mode, contactAccountId, onOpenContact }: {
  activity: OverviewActivity
  language: string
  mode: EntranceMode
  /** Any account; avatars are looked up by address. */
  contactAccountId: string
  onOpenContact: NowPageProps['onOpenContact']
}) {
  const complete = activity.previousWeekComplete
  const received = compareWeeks(activity.receivedLastWeek, activity.receivedPreviousWeek, complete)
  const sent = compareWeeks(activity.sentLastWeek, activity.sentPreviousWeek, complete)
  const showSent = activity.sentLastWeek + activity.sentPreviousWeek > 0
  const { today } = activity
  return (
    <div className="flex flex-col gap-4 px-3">
      <ActivityChart days={activity.days} language={language} mode={mode} />
      <div data-slot="now-activity-lines" className="flex flex-col gap-1.5 text-sm text-muted-foreground">
        <p>{t(`now:receivedWeek.${received.kind}`, { count: activity.receivedLastWeek, diff: 'diff' in received ? received.diff : 0 })}</p>
        {showSent && <p>{t(`now:sentWeek.${sent.kind}`, { count: activity.sentLastWeek, diff: 'diff' in sent ? sent.diff : 0 })}</p>}
        {activity.topContacts.length > 0 && (
          <div className="flex flex-wrap items-center gap-x-1 gap-y-1">
            <span className="mr-1">{t('now:topContacts')}</span>
            {activity.topContacts.map((contact) => (
              <button
                key={contact.address}
                type="button"
                onClick={() => onOpenContact(contact)}
                className="flex max-w-48 items-center gap-1.5 rounded-full py-0.5 pr-2.5 pl-0.5 text-foreground outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring"
              >
                <SenderAvatar sender={{ name: contact.name, address: contact.address }} accountId={contactAccountId} size="sm" className="size-5 text-[9px]" />
                <span className="truncate">{contact.name?.trim() || contact.address}</span>
              </button>
            ))}
          </div>
        )}
        <p>
          {today.received === 0 ? t('now:todayNone')
            : today.unread > 0 ? t('now:todayUnread', { count: today.received, unread: today.unread })
              : t('now:todayAllRead', { count: today.received })}
        </p>
      </div>
    </div>
  )
}

/** Most codes listed, as the overview caps its own list. */
const OVERVIEW_CODES = 3

/** A code the shared recognizer confirmed, whether the overview found it or only suspected it. */
type ShownCode = Pick<OverviewCode, 'emailId' | 'accountId' | 'from' | 'receivedAt'> & { ready: ReadyVerificationCode }

/** Asks the shared controller to recognize the codes and candidates; nothing while the page is gone. */
function useCodeTargets(verification: VerificationCodes, codes: (OverviewCode | CodeCandidate)[]) {
  const key = JSON.stringify(codes.map((code): VerificationTarget => ({ emailId: code.emailId, bodyState: code.bodyState })))
  const targets = useMemo(() => JSON.parse(key) as VerificationTarget[], [key])
  const request = verification.requestVisible
  useEffect(() => { request(targets) }, [request, targets])
  useEffect(() => () => { request([]) }, [request])
}

/**
 * One code the shared recognizer confirmed. Copying goes through the shared
 * flow: the host writes the clipboard, the move-to-Trash setting applies and
 * the notices match the list and reader.
 */
function CodeRow({ code, language, now, verification, onOpenEmail }: { code: ShownCode; language: string; now: number; verification: VerificationCodes; onOpenEmail: NowPageProps['onOpenEmail'] }) {
  const { ready } = code
  const shown = ready.code
  const busy = verification.busy(code.emailId)
  const copied = verification.copied(code.emailId, ready.recognitionId)
  const label = busy ? t('mail:verificationCopying') : copied ? t('mail:verificationCopiedShort') : t('now:copyCode', { code: shown })
  return (
    <li className="flex items-center gap-3 rounded-lg px-3 py-2">
      <p className="font-mono text-2xl font-semibold tracking-[0.18em] tabular-nums">
        <span className="sr-only">{t('now:codeLabel', { code: shown })}</span>
        <span aria-hidden>{shown}</span>
      </p>
      <div className="min-w-0 flex-1 text-sm">
        <button
          type="button"
          className="max-w-full truncate rounded-sm text-left font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => onOpenEmail({ id: code.emailId, accountId: code.accountId })}
        >
          {displayName(code.from)}
        </button>
        <p className="text-xs text-muted-foreground">{relativeTime(code.receivedAt, language, now)}</p>
      </div>
      <IconButton
        label={label}
        size="icon-sm"
        disabled={busy}
        onClick={() => void verification.copy(code.emailId, ready.recognitionId)}
      >
        <HugeiconsIcon icon={copied ? Tick02Icon : Copy01Icon} />
      </IconButton>
    </li>
  )
}

const rowClass = 'flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring'
const linkClass = 'rounded-md px-3 py-1 text-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring'

function motionProps(mode: EntranceMode, index: number, className?: string) {
  const motion = entrance(mode, index)
  return { className: cn(className, motion.className), style: motion.style }
}

function Section({ title, mode, index, children }: { title: string; mode: EntranceMode; index: number; children: ReactNode }) {
  const id = useId()
  return (
    <section aria-labelledby={id} {...motionProps(mode, index, 'flex flex-col gap-2')}>
      <h2 id={id} className="px-3 text-xs font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}

/** A small count that leads somewhere; screen readers get the label and the number once. */
function StatButton({ value, label, mode, onClick }: { value: number; label: string; mode: EntranceMode; onClick: () => void }) {
  const shown = useCountUp(value, mode === 'first')
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={`${label} ${value}`}
      className="flex items-baseline gap-1.5 rounded-lg bg-muted/40 px-3 py-1.5 text-sm outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span className="text-muted-foreground">{label}</span>
      <span className="font-semibold tabular-nums">{shown}</span>
    </button>
  )
}

function MailRow({ email, language, now, isNew, onOpen }: { email: EmailSummary; language: string; now: number; isNew: boolean; onOpen: (email: EmailSummary) => void }) {
  const subject = email.subject || t('mail:noSubject')
  return (
    <button type="button" onClick={() => onOpen(email)} className={rowClass}>
      <SenderAvatar sender={email.from} accountId={email.accountId} size="sm" />
      {/* Side by side when there is room, stacked in a narrow window. */}
      <span className="flex min-w-0 flex-1 flex-col sm:flex-row sm:items-center sm:gap-3">
        <span className="truncate font-medium sm:w-32 sm:shrink-0">{displayName(email.from)}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{subject}</span>
      </span>
      {isNew && <span data-slot="now-new-tag" className="shrink-0 rounded-sm bg-foreground/5 px-1.5 py-px text-[11px] text-muted-foreground">{t('now:newTag')}</span>}
      <span className="shrink-0 text-xs text-muted-foreground">{relativeTime(email.receivedAt, language, now)}</span>
    </button>
  )
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
