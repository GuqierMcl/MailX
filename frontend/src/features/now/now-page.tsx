import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useEffect, useId, useMemo, type ReactNode } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import { Alert02Icon, Attachment01Icon, Copy01Icon, File01Icon, MailUpload01Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import { useQuery } from '@tanstack/react-query'
import type { Account, AccountSyncState, EmailSummary, Folder, OutboxItem, OverviewCode } from '@/generated/accounts'
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
import { displayName } from '@/features/mail/format'
import type { Mailbox } from '@/features/mail/mailbox'
import { readyVerificationCode, type ReadyVerificationCode, type VerificationCodes, type VerificationTarget } from '@/features/mail/use-verification-codes'
import { daySegment, lineContext, lineKey, type LineContext, type NowKey } from './greeting'
import { useDayClock, useFirstVisitToday, useLastVisit } from './visit'
import { entrance, useCountUp } from './motion'
import { useFreshCodes } from './use-fresh-codes'
import { NowAmbient } from './now-ambient'
import { relativeTime } from './relative-time'

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
 * centered column that scrolls on its own. Sections appear only with content.
 */
export function NowPage({ client, accounts, folders, syncStates, draftCount, outbox, onSelectMailbox, onOpenAccount, onOpenEmail, verification }: NowPageProps) {
  const { i18n } = useTranslation()
  const language = i18n.language
  const now = useDayClock()
  const since = useLastVisit()
  const firstToday = useFirstVisitToday()
  const reducedMotion = usePrefersReducedMotion()
  // Only the day's first visit plays the entrance; reduced motion never does.
  const animate = firstToday && !reducedMotion
  const settings = useSettings(client)
  const overview = useQuery({
    queryKey: mailKeys.overview(since),
    queryFn: () => client.getMailOverview({ since }),
  })
  const data = overview.data
  const fresh = useFreshCodes(data?.verificationCodes ?? [])
  useCodeTargets(verification, fresh)
  // Only codes the shared recognizer has confirmed, so every one shown can be copied.
  const codes = fresh.flatMap((code) => {
    const ready = readyVerificationCode(verification.status(code.emailId))
    return ready ? [{ ...code, ready }] : []
  })

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
  const starredUnread = data?.starredUnread ?? 0
  const newMail = data?.newMail.total ?? 0
  const attachments = data?.recentAttachments ?? []

  const segment = daySegment(now)
  const context = lineContext(now, language, { attention: urgent, newMail, unread })
  const nothingPending = data !== undefined && attention.length === 0 && newMail === 0 && unread === 0
    && starredUnread === 0 && codes.length === 0 && attachments.length === 0

  const defaultId = settings.data?.settings.compose.defaultAccountId
  const name = (accounts.find((account) => account.id === defaultId) ?? accounts[0])?.senderName?.trim() || null
  const greeting = name ? t(`now:greetingNamed.${segment}`, { name }) : t(`now:greeting.${segment}`)
  const date = new Intl.DateTimeFormat(language, { month: 'long', day: 'numeric', weekday: 'long' }).format(now)

  const accountOf = (id: string) => accounts.find((account) => account.id === id)
  let index = 1
  const next = () => index++

  return (
    <div data-slot="now-page" className="relative min-h-0 flex-1 overflow-hidden bg-background">
      <NowAmbient segment={segment} />
      <div className="relative h-full overflow-y-auto">
        <div className="mx-auto flex w-full max-w-190 flex-col gap-9 px-5 pt-10 pb-16 sm:px-10 sm:pt-14">
          <header {...motionProps(animate, 0, 'px-3')}>
            <p className="text-sm text-muted-foreground">{date}</p>
            <h1 className="mt-1 text-3xl font-semibold tracking-tight text-balance">{greeting}</h1>
            {/* The line waits for the overview, so it does not change as soon as it appears. */}
            <p className="mt-2 min-h-6 text-base text-muted-foreground">{data ? summaryLine(now, context) : null}</p>
            {nothingPending && context.kind !== 'clear' && <p className="mt-1 text-base text-muted-foreground">{t('now:allClear')}</p>}
          </header>

          {attention.length > 0 && (
            <Section title={t('now:attention')} animate={animate} index={next()}>
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

          {data && newMail > 0 && (
            <Section title={t('now:newMail')} animate={animate} index={next()}>
              <div className="flex flex-wrap items-end gap-x-6 gap-y-3 px-3">
                <BigNumber value={newMail} label={t('now:newMailTotal', { count: newMail })} animate={animate} />
                {data.newMail.accounts.length > 1 && (
                  <ul className="flex flex-wrap gap-x-4 gap-y-1 pb-1.5 text-sm text-muted-foreground">
                    {data.newMail.accounts.map(({ accountId, count }) => {
                      const account = accountOf(accountId)
                      if (!account) return null
                      return (
                        <li key={accountId} className="flex items-center gap-1.5" aria-label={t('now:accountCount', { account: account.displayName, count })}>
                          <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-4 text-[9px]" />
                          <span aria-hidden className="max-w-40 truncate">{account.displayName}</span>
                          <span aria-hidden className="tabular-nums text-foreground">{count}</span>
                        </li>
                      )
                    })}
                  </ul>
                )}
              </div>
              <ul className="flex flex-col">
                {data.newMail.latest.map((email) => (
                  <li key={email.id}>
                    <MailRow email={email} language={language} onOpen={onOpenEmail} />
                  </li>
                ))}
              </ul>
            </Section>
          )}

          {data && (unread > 0 || starredUnread > 0) && (
            <Section title={t('now:unreadStarred')} animate={animate} index={next()}>
              <div className="flex flex-wrap gap-2">
                {unread > 0 && (
                  <StatButton value={unread} label={t('now:inboxUnread')} animate={animate} onClick={() => onSelectMailbox({ kind: 'unified', role: 'inbox' })} />
                )}
                {starredUnread > 0 && (
                  <StatButton value={starredUnread} label={t('now:starredUnread')} animate={animate} onClick={() => onSelectMailbox({ kind: 'starred' })} />
                )}
              </div>
            </Section>
          )}

          {codes.length > 0 && (
            <Section title={t('now:codes')} animate={animate} index={next()}>
              <ul className="flex flex-col rounded-xl bg-muted/40 p-1">
                {codes.map((code) => (
                  <CodeRow key={code.emailId} code={code} language={language} verification={verification} onOpenEmail={onOpenEmail} />
                ))}
              </ul>
            </Section>
          )}

          {attachments.length > 0 && (
            <Section title={t('now:attachments')} animate={animate} index={next()}>
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
                      <span className="shrink-0 text-xs text-muted-foreground">{relativeTime(email.receivedAt, language)}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </Section>
          )}
        </div>
      </div>
    </div>
  )
}

/** Asks the shared controller to recognize the shown codes; nothing while the page is gone. */
function useCodeTargets(verification: VerificationCodes, codes: OverviewCode[]) {
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
function CodeRow({ code, language, verification, onOpenEmail }: { code: OverviewCode & { ready: ReadyVerificationCode }; language: string; verification: VerificationCodes; onOpenEmail: NowPageProps['onOpenEmail'] }) {
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
        <p className="text-xs text-muted-foreground">{relativeTime(code.receivedAt, language)}</p>
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

function motionProps(animate: boolean, index: number, className?: string) {
  const motion = entrance(animate, index)
  return { className: cn(className, motion.className), style: motion.style }
}

function Section({ title, animate, index, children }: { title: string; animate: boolean; index: number; children: ReactNode }) {
  const id = useId()
  return (
    <section aria-labelledby={id} {...motionProps(animate, index, 'flex flex-col gap-2')}>
      <h2 id={id} className="px-3 text-xs font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}

/** A large number; screen readers get the sentence, not each counting frame. */
function BigNumber({ value, label, animate }: { value: number; label: string; animate: boolean }) {
  const shown = useCountUp(value, animate)
  return (
    <p className="text-4xl leading-none font-semibold tracking-tight tabular-nums">
      <span className="sr-only">{label}</span>
      <span aria-hidden>{shown}</span>
    </p>
  )
}

function StatButton({ value, label, animate, onClick }: { value: number; label: string; animate: boolean; onClick: () => void }) {
  const shown = useCountUp(value, animate)
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={`${label} ${value}`}
      className="flex min-w-36 flex-col items-start gap-1 rounded-xl bg-muted/40 px-4 py-3 text-left outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span className="text-2xl leading-none font-semibold tabular-nums">{shown}</span>
      <span className="text-sm text-muted-foreground">{label}</span>
    </button>
  )
}

function MailRow({ email, language, onOpen }: { email: EmailSummary; language: string; onOpen: (email: EmailSummary) => void }) {
  const subject = email.subject || t('mail:noSubject')
  return (
    <button type="button" onClick={() => onOpen(email)} className={rowClass}>
      <SenderAvatar sender={email.from} accountId={email.accountId} size="sm" />
      {/* Side by side when there is room, stacked in a narrow window. */}
      <span className="flex min-w-0 flex-1 flex-col sm:flex-row sm:items-center sm:gap-3">
        <span className="truncate font-medium sm:w-32 sm:shrink-0">{displayName(email.from)}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{subject}</span>
      </span>
      <span className="shrink-0 text-xs text-muted-foreground">{relativeTime(email.receivedAt, language)}</span>
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
