import { useCallback, useState } from 'react'
import { act, cleanup, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Account, AccountSyncState, CodeCandidate, EmailPage, EmailSummary, Folder, MailOverview, OutboxItem, OverviewActivity, OverviewCode, VerificationCodeStatus } from '@/generated/accounts'
import { exampleAccount, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import { TooltipProvider } from '@/components/ui/tooltip'
import type { MailxClient } from '@/lib/mailx-client'
import { useVerificationCodes, type VerificationTarget } from '@/features/mail/use-verification-codes'
import { daySegment, localDay } from './greeting'
import { ENTRANCE_KEY, LAST_VISIT_KEY } from './visit'
import { NOW_AMBIENT_KEY, NOW_AMBIENT_SCENE_KEY } from './ambient-preference'
import { ATTACHMENTS_OPEN_KEY } from './modules/attachments-view'
import { dayStarts } from './activity'
import { NowPage, type NowPageProps } from './now-page'
import { SNOOZED_KEY } from './modules/snooze'
import type { NowSettings } from '@/generated/accounts'

const account = exampleAccount({ displayName: '工作邮箱', senderName: '林夕' })

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    senderGroup: null,
    id, accountId: account.id, folderId: 'inbox', subject: `邮件 ${id}`,
    from: { name: '王五', address: 'wang@example.com' }, preview: '',
    receivedAt: new Date(Date.now() - 5 * 60_000).toISOString(), isRead: false, isStarred: false,
    hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

function inbox(unreadCount: number): Folder {
  return { id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox', selectable: true, countsKnown: true, unreadCount, totalCount: unreadCount + 2, archivedCount: 0, archivedUnreadCount: 0 }
}

function page(items: EmailSummary[]): EmailPage {
  return { items, nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false, historyReachedEnd: false } }
}

function activityOf(overrides: Partial<OverviewActivity> = {}): OverviewActivity {
  return {
    days: dayStarts(new Date()).map((start, index) => ({ start, received: index === 13 ? 5 : index % 3 })),
    receivedLastWeek: 40, receivedPreviousWeek: 28, sentLastWeek: 10, sentPreviousWeek: 11, previousWeekComplete: true,
    topContacts: [{ address: 'zhao@example.com', name: '赵六', count: 6 }, { address: 'qian@example.com', name: null, count: 3 }],
    today: { received: 5, unread: 2 },
    ...overrides,
  }
}

const empty: MailOverview = { newMail: { total: 0, accounts: [], latest: [], senders: [] }, starredUnread: 0, unreadSenders: [], verificationCodes: [], codeCandidates: [], recentAttachments: [], generatedAt: new Date().toISOString() }

function overviewCode(emailId: string, code: string, overrides: Partial<OverviewCode> = {}): OverviewCode {
  return {
    emailId, accountId: account.id, folderId: 'inbox', from: { name: '银行', address: 'bank@example.com' }, code,
    receivedAt: new Date(Date.now() - 60_000).toISOString(), bodyState: 'ready', ...overrides,
  }
}

type Handlers = Pick<NowPageProps, 'onSelectMailbox' | 'onOpenAccount' | 'onOpenEmail' | 'onOpenUnread' | 'onOpenContact'>
type HarnessProps = { client: MailxClient; accounts: Account[]; handlers: Handlers; props: Partial<NowPageProps>; requested: (targets: VerificationTarget[]) => void }

/** The workbench's shared controller, kept mounted while the page can be closed. */
function Harness({ client, accounts, handlers, props, requested }: HarnessProps) {
  const controller = useVerificationCodes(client, null, 'test', vi.fn())
  const forward = controller.requestVisible
  const requestVisible = useCallback((targets: VerificationTarget[]) => { requested(targets); forward(targets) }, [forward, requested])
  const [open, setOpen] = useState(true)
  return (
    <>
      <button type="button" onClick={() => setOpen(false)}>close now page</button>
      {open && <NowPage client={client} accounts={accounts} folders={[]} syncStates={{}} draftCount={0} outbox={[]} verification={{ ...controller, requestVisible }} {...handlers} {...props} />}
    </>
  )
}

function setup({ overview = empty, unread = [], accounts = [account], props = {}, statuses = {}, now }: {
  /** Saved module preferences. */
  now?: Partial<NowSettings>
  overview?: MailOverview
  /** What the unread list query returns. */
  unread?: EmailSummary[]
  accounts?: Account[]
  props?: Partial<NowPageProps>
  statuses?: Record<string, Omit<VerificationCodeStatus, 'emailId'>>
} = {}) {
  const client = testClient(accounts)
  client.getMailOverview.mockResolvedValue(overview)
  client.listEmails.mockResolvedValue(page(unread))
  if (now) void client.updateSettings({ now: { arrangement: 'auto', order: [], hidden: [], ...now } })
  client.getVerificationCode.mockImplementation(async (emailId) => ({ emailId, ...(statuses[emailId] ?? { state: 'notFound', code: null, recognitionId: null }) }))
  const handlers = { onSelectMailbox: vi.fn(), onOpenAccount: vi.fn(), onOpenEmail: vi.fn(), onOpenUnread: vi.fn(), onOpenContact: vi.fn() }
  const requested = vi.fn<(targets: VerificationTarget[]) => void>()
  render(
    <TooltipProvider>
      <Harness client={client} accounts={accounts} handlers={handlers} props={props} requested={requested} />
      <Toaster />
    </TooltipProvider>,
  )
  return { client, requested, ...handlers }
}

const ready = (code: string, recognitionId = `r-${code}`) => ({ state: 'ready' as const, code, recognitionId })
const pending = (state: 'unchecked' | 'unavailable' = 'unchecked') => ({ state, code: null, recognitionId: null })

/** Waits for the overview: the summary line appears with it. */
const loaded = async () => {
  const heading = await screen.findByRole('heading', { level: 1 })
  await waitFor(() => expect(heading.parentElement!.querySelectorAll('p')[1]).not.toBeEmptyDOMElement())
}

const synced = (overrides: Partial<AccountSyncState> = {}) => ({ [account.id]: exampleSyncState({ accountId: account.id, lastSyncedAt: new Date(Date.now() - 2 * 60_000).toISOString(), ...overrides }) })

afterEach(() => {
  localStorage.clear()
  vi.restoreAllMocks()
})

describe('Now page', () => {
  it('greets by name and shows only the all-clear line when nothing is pending', async () => {
    const { client } = setup()
    await loaded()
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(/林夕/)
    expect(screen.queryAllByRole('heading', { level: 2 })).toEqual([])
    // The first visit counts from the start of today; the chart asks for the last fourteen local days.
    const start = new Date()
    start.setHours(0, 0, 0, 0)
    expect(client.getMailOverview).toHaveBeenCalledWith({ since: start.toISOString(), dayStarts: dayStarts(new Date()) })
  })

  it('greets without a name when the sending account has none', async () => {
    setup({ accounts: [exampleAccount({ senderName: null })] })
    await loaded()
    expect(screen.getByRole('heading', { level: 1 }).textContent).not.toMatch(/，/)
  })

  it('shows only the backdrop until the overview and the unread list have answered', async () => {
    const client = testClient([account])
    let answerOverview!: (overview: MailOverview) => void
    let failList!: (error: Error) => void
    client.getMailOverview.mockReturnValue(new Promise((resolve) => { answerOverview = resolve }))
    client.listEmails.mockReturnValue(new Promise((_, reject) => { failList = reject }))
    const view = render(
      <TooltipProvider>
        <NowPage client={client} accounts={[account]} folders={[]} syncStates={{}} draftCount={0} outbox={[]} verification={stubVerification()}
          onSelectMailbox={vi.fn()} onOpenUnread={vi.fn()} onOpenContact={vi.fn()} onOpenAccount={vi.fn()} onOpenEmail={vi.fn()} />
      </TooltipProvider>,
    )
    await waitFor(() => expect(client.listEmails).toHaveBeenCalled())
    expect(view.container.querySelector('[data-slot="now-ambient"]')).toBeInTheDocument()
    expect(screen.queryByRole('heading', { level: 1 })).not.toBeInTheDocument()
    await act(async () => answerOverview(empty))
    expect(screen.queryByRole('heading', { level: 1 })).not.toBeInTheDocument()
    // A failed list still lets the page show.
    await act(async () => failList(new Error('offline')))
    expect(await screen.findByRole('heading', { level: 1 })).toBeInTheDocument()
  })

  it('lists what needs attention and opens where it is handled', async () => {
    const stopped = exampleAccount({ id: 'stopped', displayName: '旧邮箱' })
    const failed = { id: 'o1', state: 'failed' } as OutboxItem
    const { onOpenAccount, onSelectMailbox } = setup({
      accounts: [stopped],
      props: {
        syncStates: { stopped: exampleSyncState({ accountId: 'stopped', failure: { code: 'network', message: '', detail: null, willRetry: false } }) },
        outbox: [failed],
        draftCount: 2,
      },
    })
    await loaded()
    const section = screen.getByRole('region', { name: '需要处理' })
    await userEvent.click(within(section).getByRole('button', { name: /旧邮箱 已停止同步/ }))
    expect(onOpenAccount).toHaveBeenCalledWith('stopped')
    await userEvent.click(within(section).getByRole('button', { name: '1 封邮件发送失败' }))
    expect(onSelectMailbox).toHaveBeenLastCalledWith({ kind: 'outbox' })
    // Drafts are a module of their own, right after the codes.
    const drafts = screen.getByRole('region', { name: '草稿' })
    await userEvent.click(within(drafts).getByRole('button', { name: '2 份草稿未完成' }))
    expect(onSelectMailbox).toHaveBeenLastCalledWith({ kind: 'drafts' })
  })

  describe('inbox', () => {
    it('opens new mail, unread with the Unread filter, and starred unread', async () => {
      const { onSelectMailbox, onOpenUnread } = setup({
        overview: { ...empty, newMail: { total: 3, accounts: [{ accountId: account.id, count: 3 }], latest: [], senders: [] }, starredUnread: 2 },
        props: { folders: [inbox(7)] },
      })
      const section = await screen.findByRole('region', { name: '收件箱' })
      await userEvent.click(within(section).getByRole('button', { name: '新邮件 3' }))
      expect(onSelectMailbox).toHaveBeenLastCalledWith({ kind: 'unified', role: 'inbox' })
      await userEvent.click(within(section).getByRole('button', { name: '未读 7' }))
      expect(onOpenUnread).toHaveBeenCalledTimes(1)
      await userEvent.click(within(section).getByRole('button', { name: '星标未读 2' }))
      expect(onSelectMailbox).toHaveBeenLastCalledWith({ kind: 'starred' })
    })

    it('hides empty counts but keeps unread, even at zero', async () => {
      setup({ overview: { ...empty, starredUnread: 1 }, props: { folders: [inbox(0)] } })
      const section = await screen.findByRole('region', { name: '收件箱' })
      expect(within(section).getByRole('button', { name: '未读 0' })).toBeInTheDocument()
      expect(within(section).queryByRole('button', { name: /^新邮件/ })).not.toBeInTheDocument()
    })

    it('is absent with nothing new, unread or starred', async () => {
      setup({ props: { folders: [inbox(0)] } })
      await loaded()
      expect(screen.queryByRole('region', { name: '收件箱' })).not.toBeInTheDocument()
    })

    it('lists at most six unread messages, marks the ones since the last visit, and opens one', async () => {
      const since = new Date(Date.now() - 3_600_000).toISOString()
      localStorage.setItem(LAST_VISIT_KEY, since)
      const rows = Array.from({ length: 8 }, (_, index) => email(`u${index}`, {
        subject: `未读 ${index}`,
        // The first two arrived after the last visit.
        receivedAt: new Date(Date.now() - (index < 2 ? 10 : 120) * 60_000 - index * 1000).toISOString(),
      }))
      const { client, onOpenEmail } = setup({ unread: rows, props: { folders: [inbox(6)] } })
      const section = await screen.findByRole('region', { name: '收件箱' })
      expect(client.listEmails).toHaveBeenCalledWith(expect.objectContaining({ folderRole: 'inbox', archive: 'exclude', unread: true, limit: 6, text: null, groupSenders: false }))
      const list = within(section).getByRole('list')
      expect(within(list).getAllByRole('listitem')).toHaveLength(6)
      expect(within(list).getAllByText('新')).toHaveLength(2)
      expect(within(within(list).getAllByRole('listitem')[2]).queryByText('新')).not.toBeInTheDocument()
      // Six unread: nothing more to see.
      expect(within(section).queryByRole('button', { name: /查看全部/ })).not.toBeInTheDocument()
      await userEvent.click(within(section).getByRole('button', { name: /未读 1/ }))
      expect(onOpenEmail).toHaveBeenCalledWith(rows[1])
    })

    it('links to every unread message above six', async () => {
      const { onOpenUnread } = setup({ unread: Array.from({ length: 6 }, (_, index) => email(`u${index}`)), props: { folders: [inbox(23)] } })
      const section = await screen.findByRole('region', { name: '收件箱' })
      await userEvent.click(within(section).getByRole('button', { name: '查看全部 23 封未读' }))
      expect(onOpenUnread).toHaveBeenCalled()
    })
  })

  describe('recent activity', () => {
    it('charts fourteen days with today emphasized and labeled for assistive technology', async () => {
      const activity = activityOf()
      setup({ overview: { ...empty, activity } })
      const section = await screen.findByRole('region', { name: '近况' })
      const chart = within(section).getByRole('group', { name: '近 14 天每天收到的邮件' })
      const columns = within(chart).getAllByRole('img')
      expect(columns).toHaveLength(14)
      const today = columns[13]
      expect(today).toHaveAttribute('data-today')
      expect(today.querySelector('[data-slot="activity-bar"]')).toHaveClass('bg-primary')
      expect(columns[0].querySelector('[data-slot="activity-bar"]')).not.toHaveClass('bg-primary')
      // A day without mail is a 2px stub at the baseline.
      expect(columns[0].querySelector<HTMLElement>('[data-slot="activity-bar"]')!.style.height).toBe('2px')
      const date = new Date(activity.days[13].start)
      const expected = `${new Intl.DateTimeFormat('zh-CN', { month: 'short', day: 'numeric' }).format(date)} ${new Intl.DateTimeFormat('zh-CN', { weekday: 'short' }).format(date)} · 5 封`
      expect(today).toHaveAccessibleName(expected)
      expect(within(section).getByText('14 天前')).toBeInTheDocument()
      expect(within(section).getByText('今天')).toBeInTheDocument()
    })

    it('shows a day in a tooltip on keyboard focus and moves with the arrow keys', async () => {
      setup({ overview: { ...empty, activity: activityOf() } })
      const chart = await screen.findByRole('group', { name: '近 14 天每天收到的邮件' })
      const columns = within(chart).getAllByRole('img')
      // One tab stop for the chart, on today.
      expect(columns.filter((column) => column.tabIndex === 0)).toEqual([columns[13]])
      for (let i = 0; i < 20 && document.activeElement !== columns[13]; i++) await userEvent.tab()
      expect(columns[13]).toHaveFocus()
      const name = columns[13].getAttribute('aria-label')!
      expect(await screen.findByText(name, { selector: '[data-slot="tooltip-content"]' })).toBeInTheDocument()
      await userEvent.keyboard('{ArrowLeft}')
      expect(columns[12]).toHaveFocus()
      await userEvent.keyboard('{Home}')
      expect(columns[0]).toHaveFocus()
    })

    it('compares the weeks: more, about the same, and only the count while the earlier week is incomplete', async () => {
      setup({ overview: { ...empty, activity: activityOf() } })
      const section = await screen.findByRole('region', { name: '近况' })
      expect(within(section).getByText('近 7 天收到 40 封，比前 7 天多 12 封')).toBeInTheDocument()
      expect(within(section).getByText('发出 10 封，和前 7 天差不多')).toBeInTheDocument()
    })

    it('states only the counts when the earlier week is not fully cached', async () => {
      setup({ overview: { ...empty, activity: activityOf({ previousWeekComplete: false }) } })
      const section = await screen.findByRole('region', { name: '近况' })
      expect(within(section).getByText('近 7 天收到 40 封')).toBeInTheDocument()
      expect(within(section).getByText('发出 10 封')).toBeInTheDocument()
    })

    it('leaves out the sent line when nothing was sent in either week', async () => {
      setup({ overview: { ...empty, activity: activityOf({ sentLastWeek: 0, sentPreviousWeek: 0, topContacts: [] }) } })
      const section = await screen.findByRole('region', { name: '近况' })
      expect(within(section).queryByText(/^发出/)).not.toBeInTheDocument()
      expect(within(section).queryByText('往来最多：')).not.toBeInTheDocument()
    })

    it('is absent when the two weeks hold no mail at all', async () => {
      const quiet = activityOf({ sentLastWeek: 0, sentPreviousWeek: 0, receivedLastWeek: 0, receivedPreviousWeek: 0, topContacts: [], today: { received: 0, unread: 0 } })
      setup({ overview: { ...empty, activity: { ...quiet, days: quiet.days.map((day) => ({ ...day, received: 0 })) } } })
      await loaded()
      expect(screen.queryByRole('region', { name: '近况' })).not.toBeInTheDocument()
    })

    it('opens a frequent contact in the contact view', async () => {
      const activity = activityOf()
      const { onOpenContact } = setup({ overview: { ...empty, activity } })
      const section = await screen.findByRole('region', { name: '近况' })
      expect(within(section).getByText('往来最多：')).toBeInTheDocument()
      await userEvent.click(within(section).getByRole('button', { name: /赵六/ }))
      expect(onOpenContact).toHaveBeenCalledWith(activity.topContacts[0])
      // Without a name, the address.
      await userEvent.click(within(section).getByRole('button', { name: /qian@example\.com/ }))
      expect(onOpenContact).toHaveBeenLastCalledWith(activity.topContacts[1])
    })

    it.each([
      [{ received: 18, unread: 2 }, '今天收到 18 封，还剩 2 封未读'],
      [{ received: 18, unread: 0 }, '今天收到 18 封，都已读过'],
      [{ received: 0, unread: 0 }, '今天还没有收到邮件'],
    ])('describes today: %o', async (today, line) => {
      setup({ overview: { ...empty, activity: activityOf({ today }) } })
      const section = await screen.findByRole('region', { name: '近况' })
      expect(within(section).getByText(line)).toBeInTheDocument()
    })
  })

  describe('sync line', () => {
    const footer = () => document.querySelector('[data-slot="now-sync"]')

    it('gives the oldest last sync with a check mark when every mailbox is synced', async () => {
      const other = exampleAccount({ id: 'other' })
      setup({
        accounts: [account, other],
        props: { syncStates: { ...synced(), other: exampleSyncState({ accountId: 'other', lastSyncedAt: new Date(Date.now() - 30_000).toISOString() }) } },
      })
      await loaded()
      expect(footer()).toHaveTextContent('所有邮箱已同步 · 2分钟前')
      expect(footer()!.querySelector('[data-slot="now-sync-icon"]')).toHaveAttribute('aria-hidden', 'true')
    })

    it('says syncing, without an icon, while a round runs', async () => {
      setup({ props: { syncStates: synced({ phase: 'syncing' }) } })
      await loaded()
      expect(footer()).toHaveTextContent('正在同步…')
      expect(footer()!.querySelector('svg')).toBeNull()
    })

    it('reports a sync problem, without an icon', async () => {
      // The failure has lasted past the quiet window, so the line reports it.
      setup({ props: { syncStates: synced({ phase: 'failed', failure: { code: 'network', message: '', detail: null, willRetry: true }, failureSince: new Date(Date.now() - 60_000).toISOString() }) } })
      await loaded()
      expect(footer()).toHaveTextContent('部分邮箱同步异常')
      expect(footer()!.querySelector('svg')).toBeNull()
    })

    it('is absent before the accounts report their state', async () => {
      setup()
      await loaded()
      expect(footer()).toBeNull()
    })
  })

  it('copies a recognized code through the shared flow and opens its message from the sender', async () => {
    const writeText = vi.fn()
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    const { client, onOpenEmail } = setup({
      overview: { ...empty, verificationCodes: [overviewCode('c1', '482913')] },
      // The shared recognizer's code wins over the overview's.
      statuses: { c1: ready('482914', 'rec-1') },
    })
    const section = await screen.findByRole('region', { name: '验证码' })
    expect(await within(section).findByText('验证码 482914')).toBeInTheDocument()
    await userEvent.click(within(section).getByRole('button', { name: '复制验证码 482914' }))
    expect(client.copyVerificationCode).toHaveBeenCalledWith('c1', 'rec-1')
    expect(await screen.findByText('验证码已复制')).toBeInTheDocument()
    expect(within(section).getByRole('button', { name: '已复制' })).toBeInTheDocument()
    expect(writeText).not.toHaveBeenCalled()
    await userEvent.click(within(section).getByRole('button', { name: '银行' }))
    expect(onOpenEmail).toHaveBeenCalledWith({ id: 'c1', accountId: account.id })
  })

  it('asks the shared recognizer for the shown codes and lets go when the page closes', async () => {
    const { client, requested } = setup({
      overview: { ...empty, verificationCodes: [overviewCode('c1', '482913'), overviewCode('c2', '7391', { bodyState: 'missing' })] },
      statuses: { c1: ready('482913'), c2: ready('7391') },
    })
    await screen.findByRole('region', { name: '验证码' })
    await waitFor(() => expect(requested).toHaveBeenLastCalledWith([{ emailId: 'c1', bodyState: 'ready' }, { emailId: 'c2', bodyState: 'missing' }]))
    // The page asks like a visible list row, so a missing body may be fetched.
    await waitFor(() => expect(client.getVerificationCode).toHaveBeenCalledWith('c2', true))
    await userEvent.click(screen.getByRole('button', { name: 'close now page' }))
    expect(requested).toHaveBeenLastCalledWith([])
  })

  it('hides codes the shared recognizer does not confirm', async () => {
    setup({
      overview: { ...empty, verificationCodes: [overviewCode('c1', '111111'), overviewCode('c2', '222222'), overviewCode('c3', '333333')] },
      statuses: { c1: { state: 'notFound', code: null, recognitionId: null }, c2: { state: 'ambiguous', code: null, recognitionId: null }, c3: ready('333333') },
    })
    const section = await screen.findByRole('region', { name: '验证码' })
    await waitFor(() => expect(within(section).queryByText('验证码 111111')).not.toBeInTheDocument())
    await waitFor(() => expect(within(section).queryByText('验证码 222222')).not.toBeInTheDocument())
    expect(within(section).getByText('验证码 333333')).toBeInTheDocument()
  })

  it('shows a code only once it can be copied', async () => {
    let finish!: (status: VerificationCodeStatus) => void
    const { client } = setup({
      overview: { ...empty, starredUnread: 1, verificationCodes: [overviewCode('c1', '482913'), overviewCode('c2', '555123'), overviewCode('c3', '777888')] },
    })
    client.getVerificationCode.mockImplementation((emailId) => emailId === 'c1'
      ? new Promise((resolve) => { finish = resolve })
      : Promise.resolve({ emailId, ...pending(emailId === 'c2' ? 'unavailable' : 'unchecked') }))
    await screen.findByRole('region', { name: '收件箱' })
    await waitFor(() => expect(finish).toBeDefined())
    // Still being checked, unreadable, or not checked without the body: nothing to copy, so nothing shown.
    expect(screen.queryByRole('region', { name: '验证码' })).not.toBeInTheDocument()
    await act(async () => finish({ emailId: 'c1', ...ready('482913') }))
    const section = await screen.findByRole('region', { name: '验证码' })
    expect(within(section).getByRole('button', { name: '复制验证码 482913' })).toBeEnabled()
    expect(within(section).queryByText('验证码 555123')).not.toBeInTheDocument()
    expect(within(section).queryByText('验证码 777888')).not.toBeInTheDocument()
    expect(client.copyVerificationCode).not.toHaveBeenCalled()
  })

  describe('code candidates', () => {
    const candidate = (emailId: string, overrides: Partial<CodeCandidate> = {}): CodeCandidate => ({
      emailId, accountId: account.id, folderId: 'inbox', from: { name: '云服务', address: 'login@cloud.example' },
      receivedAt: new Date(Date.now() - 30_000).toISOString(), bodyState: 'missing', ...overrides,
    })

    it('shows a mail with no preview once the shared recognizer answers ready, and asks it to fetch', async () => {
      const { client, requested } = setup({
        overview: { ...empty, starredUnread: 1, codeCandidates: [candidate('n1')] },
        statuses: { n1: ready('706152', 'rec-n1') },
      })
      const section = await screen.findByRole('region', { name: '验证码' })
      expect(await within(section).findByText('验证码 706152')).toBeInTheDocument()
      expect(within(section).getByRole('button', { name: '云服务' })).toBeInTheDocument()
      expect(requested).toHaveBeenCalledWith([{ emailId: 'n1', bodyState: 'missing' }])
      expect(client.getVerificationCode).toHaveBeenCalledWith('n1', true)
    })

    it('shows nothing for a candidate the recognizer does not confirm', async () => {
      const { client } = setup({
        overview: { ...empty, starredUnread: 1, codeCandidates: [candidate('n1'), candidate('n2'), candidate('n3')] },
        statuses: { n1: pending('unchecked'), n2: { state: 'notFound', code: null, recognitionId: null }, n3: { state: 'ambiguous', code: null, recognitionId: null } },
      })
      await screen.findByRole('region', { name: '收件箱' })
      await waitFor(() => expect(client.getVerificationCode).toHaveBeenCalledTimes(3))
      await act(async () => { await Promise.resolve() })
      expect(screen.queryByRole('region', { name: '验证码' })).not.toBeInTheDocument()
    })

    it('lists codes and candidates together, newest first, and drops a candidate after fifteen minutes', async () => {
      setup({
        overview: {
          ...empty,
          verificationCodes: [overviewCode('c1', '482913', { receivedAt: new Date(Date.now() - 5 * 60_000).toISOString() })],
          codeCandidates: [candidate('n1', { receivedAt: new Date(Date.now() - 60_000).toISOString() }), candidate('n2', { receivedAt: new Date(Date.now() - 16 * 60_000).toISOString() })],
        },
        statuses: { c1: ready('482913'), n1: ready('706152'), n2: ready('999999') },
      })
      const section = await screen.findByRole('region', { name: '验证码' })
      await within(section).findByText('验证码 706152')
      const codes = within(section).getAllByText(/^验证码 \d+$/).map((node) => node.textContent)
      expect(codes).toEqual(['验证码 706152', '验证码 482913'])
      expect(within(section).queryByText('验证码 999999')).not.toBeInTheDocument()
    })
  })

  it('hides a code that is already older than fifteen minutes', async () => {
    setup({
      overview: { ...empty, starredUnread: 1, verificationCodes: [overviewCode('c1', '111111', { receivedAt: new Date(Date.now() - 16 * 60_000).toISOString() })] },
    })
    await screen.findByRole('region', { name: '收件箱' })
    expect(screen.queryByRole('region', { name: '验证码' })).not.toBeInTheDocument()
  })

  describe('recent attachments', () => {
    const overview = () => {
      const named = email('a1', { hasAttachment: true })
      const unnamed = email('a2', { hasAttachment: true })
      return { named, unnamed, overview: { ...empty, recentAttachments: [{ email: named, fileNames: ['合同.pdf', '附录.docx'] }, { email: unnamed, fileNames: [] }] } }
    }

    it('is one muted line without a title, collapsed until opened', async () => {
      setup({ overview: overview().overview })
      const section = await screen.findByRole('region', { name: '最近的附件' })
      expect(within(section).queryByRole('heading')).not.toBeInTheDocument()
      const toggle = within(section).getByRole('button', { name: '最近 7 天有 2 封邮件带附件' })
      expect(toggle).toHaveAttribute('aria-expanded', 'false')
      expect(within(section).queryByText('合同.pdf')).not.toBeInTheDocument()
      // Its menu is still there for the keyboard.
      expect(within(section).getByRole('button', { name: '「最近的附件」的更多操作' })).toBeInTheDocument()
    })

    it('opens to the list, by file name or a generic label, and remembers it', async () => {
      const { named, unnamed, overview: data } = overview()
      const { onOpenEmail } = setup({ overview: data })
      const section = await screen.findByRole('region', { name: '最近的附件' })
      await userEvent.click(within(section).getByRole('button', { name: /^最近 7 天有/ }))
      expect(within(section).getByRole('button', { name: /^最近 7 天有/ })).toHaveAttribute('aria-expanded', 'true')
      expect(within(section).getByText('合同.pdf')).toBeInTheDocument()
      expect(within(section).getByText('等 1 个文件')).toBeInTheDocument()
      expect(localStorage.getItem(ATTACHMENTS_OPEN_KEY)).toBe('1')
      await userEvent.click(within(section).getByRole('button', { name: /^附件/ }))
      expect(onOpenEmail).toHaveBeenCalledWith(unnamed)
      expect(named.id).toBe('a1')
      // The next visit starts open; closing is remembered too.
      cleanup()
      setup({ overview: data })
      const again = await screen.findByRole('region', { name: '最近的附件' })
      expect(within(again).getByRole('button', { name: /^最近 7 天有/ })).toHaveAttribute('aria-expanded', 'true')
      await userEvent.click(within(again).getByRole('button', { name: /^最近 7 天有/ }))
      expect(localStorage.getItem(ATTACHMENTS_OPEN_KEY)).toBe('0')
    })

    it('says "at least" when the overview list is full', async () => {
      const rows = Array.from({ length: 5 }, (_, index) => ({ email: email(`a${index}`, { hasAttachment: true }), fileNames: [] }))
      setup({ overview: { ...empty, recentAttachments: rows } })
      expect(await screen.findByRole('button', { name: '最近 7 天至少有 5 封邮件带附件' })).toHaveAttribute('aria-expanded', 'false')
    })
  })

  describe('while you were away', () => {
    const SENDERS = [{ address: 'ann@example.com', name: 'Ann', count: 7 }, { address: 'bob@example.com', name: null, count: 3 }]
    const away = (total = 12) => ({ ...empty, newMail: { total, accounts: [], latest: [], senders: SENDERS } })
    const hoursAgo = (hours: number) => localStorage.setItem(LAST_VISIT_KEY, new Date(Date.now() - hours * 3_600_000).toISOString())

    it('says how long and how many, lists who wrote most, and opens that person', async () => {
      hoursAgo(5)
      const { onOpenContact } = setup({ overview: away() })
      const section = await screen.findByRole('region', { name: '你不在的时候' })
      expect(within(section).getByText('离开了 5 小时，来了 12 封新邮件')).toBeInTheDocument()
      expect(within(section).getByText('7 封')).toBeInTheDocument()
      // No name: the address stands in.
      expect(within(section).getByText('bob@example.com')).toBeInTheDocument()
      await userEvent.click(within(section).getByRole('button', { name: /Ann/ }))
      expect(onOpenContact).toHaveBeenCalledWith({ address: 'ann@example.com', name: 'Ann' })
    })

    it('counts days once the time is long', async () => {
      hoursAgo(80)
      setup({ overview: away(4) })
      expect(await screen.findByText('离开了 3 天，来了 4 封新邮件')).toBeInTheDocument()
    })

    it('stays away for under three hours, and without new mail', async () => {
      hoursAgo(2)
      setup({ overview: away() })
      await screen.findByRole('region', { name: '收件箱' })
      expect(screen.queryByRole('region', { name: '你不在的时候' })).not.toBeInTheDocument()
      cleanup()
      hoursAgo(9)
      setup({ overview: away(0) })
      await loaded()
      expect(screen.queryByRole('region', { name: '你不在的时候' })).not.toBeInTheDocument()
    })

    it('can be hidden like the others', async () => {
      hoursAgo(5)
      setup({ overview: away(), now: { hidden: ['away'] } })
      await screen.findByRole('region', { name: '收件箱' })
      expect(screen.queryByRole('region', { name: '你不在的时候' })).not.toBeInTheDocument()
    })
  })

  describe('clean up', () => {
    const SHOP = { address: 'shop@example.com', name: 'Shop', count: 4 }
    const overview = { ...empty, unreadSenders: [SHOP, { address: 'news@example.com', name: null, count: 3 }] }
    const item = (id: string) => ({ id, accountId: account.id, folderId: 'inbox', isRead: false, isStarred: false, archived: false, localOnly: false })

    it('lists the senders with their unread counts', async () => {
      setup({ overview })
      const section = await screen.findByRole('region', { name: '一键清理' })
      expect(within(section).getByText('Shop')).toBeInTheDocument()
      expect(within(section).getByText('4 封未读')).toBeInTheDocument()
      expect(within(section).getByText('news@example.com')).toBeInTheDocument()
    })

    it("opens the sender's mail", async () => {
      const { onOpenContact } = setup({ overview })
      await userEvent.click(await screen.findByRole('button', { name: '查看Shop的来信' }))
      expect(onOpenContact).toHaveBeenCalledWith({ address: 'shop@example.com', name: 'Shop' })
    })

    it("marks the sender's unread mail read through the bulk write-back, and undoes it", async () => {
      const { client } = setup({ overview })
      client.selectEmails.mockResolvedValue({ items: ['u1', 'u2', 'u3', 'u4'].map(item), truncated: false })
      await userEvent.click(await screen.findByRole('button', { name: '全部标为已读：Shop' }))
      await waitFor(() => expect(client.setFlags).toHaveBeenCalledWith(['u1', 'u2', 'u3', 'u4'], 'read', true))
      expect(client.selectEmails).toHaveBeenCalledWith(expect.objectContaining({ folderRole: 'inbox', archive: 'exclude', unread: true, fromAddress: 'shop@example.com' }))
      expect(await screen.findByText('已将 4 封标为已读')).toBeInTheDocument()
      await userEvent.click(screen.getByRole('button', { name: '撤销' }))
      await waitFor(() => expect(client.setFlags).toHaveBeenLastCalledWith(['u1', 'u2', 'u3', 'u4'], 'read', false))
    })

    it('says so when the write fails', async () => {
      const { client } = setup({ overview })
      client.selectEmails.mockResolvedValue({ items: [item('u1')], truncated: false })
      client.setFlags.mockRejectedValueOnce(new Error('boom'))
      await userEvent.click(await screen.findByRole('button', { name: '全部标为已读：Shop' }))
      expect((await screen.findAllByText('无法更改已读状态')).length).toBeGreaterThan(0)
    })
  })

  it('refreshes relative times once a minute', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      setup({ unread: [email('m1', { subject: '季度报告', receivedAt: new Date(Date.now() - 2 * 60_000).toISOString() })], props: { folders: [inbox(1)] } })
      const section = await screen.findByRole('region', { name: '收件箱' })
      const row = within(section).getByRole('button', { name: /季度报告/ })
      expect(row).toHaveTextContent('2分钟前')
      await act(async () => { vi.advanceTimersByTime(60_000) })
      expect(row).toHaveTextContent('3分钟前')
    } finally {
      vi.useRealTimers()
    }
  })

  it('plays the entrance on the first visit of the day only', async () => {
    setup({ overview: { ...empty, starredUnread: 2, activity: activityOf() } })
    await loaded()
    expect(screen.getByRole('heading', { level: 1 }).parentElement).toHaveClass('animate-in')
    expect(localStorage.getItem(ENTRANCE_KEY)).toBe(localDay(new Date()))
    // The bars grow from the baseline, a little after one another.
    const bars = document.querySelectorAll<HTMLElement>('[data-slot="activity-bar"]')
    expect(bars[0]).toHaveClass('animate-now-bar-grow')
    expect(bars[13].style.animationDelay).toBe('260ms')
  })

  it('plays a lighter entrance on a later visit the same day: a short rise, no count-up, quick bars', async () => {
    localStorage.setItem(ENTRANCE_KEY, localDay(new Date()))
    const raf = vi.spyOn(window, 'requestAnimationFrame')
    setup({ overview: { ...empty, newMail: { total: 12, accounts: [], latest: [], senders: [] }, starredUnread: 2, activity: activityOf() } })
    const section = await screen.findByRole('region', { name: '收件箱' })
    const header = screen.getByRole('heading', { level: 1 }).parentElement!
    expect(header).toHaveClass('animate-now-rise')
    expect(header).not.toHaveClass('animate-in')
    expect(section).toHaveClass('animate-now-rise')
    expect(section.style.animationDelay).toBe('25ms')
    // Numbers show at once.
    expect(within(section).getByRole('button', { name: '新邮件 12' })).toHaveTextContent('12')
    const bars = document.querySelectorAll<HTMLElement>('[data-slot="activity-bar"]')
    expect(bars[0]).toHaveClass('animate-now-bar-grow-quick')
    expect(bars[0]).not.toHaveClass('animate-now-bar-grow')
    expect(bars[13].style.animationDelay).toBe('104ms')
    expect(raf).not.toHaveBeenCalled()
  })

  it('shows an icon for the part of the day before the greeting, hidden from screen readers', async () => {
    setup()
    await loaded()
    const heading = screen.getByRole('heading', { level: 1 })
    const icon = heading.querySelector('[data-slot="now-greeting-icon"]')!
    expect(icon).toHaveAttribute('aria-hidden', 'true')
    expect(icon).toHaveAttribute('data-segment', daySegment(new Date()))
    // Only the current icon is opaque; the others wait at zero for a cross-fade.
    expect(icon.querySelectorAll('svg.opacity-100')).toHaveLength(1)
    expect(icon.querySelectorAll('svg.opacity-0')).toHaveLength(5)
    expect(icon).toHaveClass('animate-now-icon-in')
    // Not part of the accessible name: the greeting text already says it.
    expect(heading).toHaveAccessibleName(/^[^\s]/)
  })

  it('cross-fades the icon when the part of the day changes while the page is open', async () => {
    vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] })
    try {
      vi.setSystemTime(new Date(2026, 10, 18, 10, 59, 30))
      setup()
      await act(async () => { await vi.advanceTimersByTimeAsync(100) })
      const icon = () => document.querySelector('[data-slot="now-greeting-icon"]')
      expect(icon()).toHaveAttribute('data-segment', 'morning')
      await act(async () => { await vi.advanceTimersByTimeAsync(31_000) })
      expect(icon()).toHaveAttribute('data-segment', 'noon')
      expect(icon()!.querySelectorAll('svg.opacity-100')).toHaveLength(1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('skips the rise, stagger, count-up and growing bars with reduced motion', async () => {
    vi.spyOn(window, 'matchMedia').mockImplementation((query: string) => ({
      matches: query.includes('reduce'), media: query, onchange: null,
      addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
    }))
    const raf = vi.spyOn(window, 'requestAnimationFrame')
    setup({ overview: { ...empty, newMail: { total: 12, accounts: [], latest: [], senders: [] }, activity: activityOf() } })
    const section = await screen.findByRole('region', { name: '收件箱' })
    expect(section).not.toHaveClass('animate-in')
    expect(screen.getByRole('heading', { level: 1 }).parentElement).not.toHaveClass('animate-in')
    expect(within(section).getByRole('button', { name: '新邮件 12' })).toHaveTextContent('12')
    expect(document.querySelector('[data-slot="activity-bar"]')).not.toHaveClass('animate-now-bar-grow')
    expect(document.querySelector('[data-slot="activity-bar"]')).not.toHaveClass('animate-now-bar-grow-quick')
    expect(document.querySelector('.animate-now-rise')).not.toBeInTheDocument()
    // The icon is simply there.
    expect(document.querySelector('[data-slot="now-greeting-icon"]')).not.toHaveClass('animate-now-icon-in')
    expect(raf).not.toHaveBeenCalled()
  })

  describe('modules', () => {
    /** Names of the module regions, top to bottom. */
    const regions = () => [...document.querySelectorAll('[data-slot="now-content"] > section')].map((section) => section.getAttribute('aria-label') ?? section.querySelector('h2')?.textContent)
    const everything = {
      overview: { ...empty, verificationCodes: [overviewCode('c1', '482913')], newMail: { total: 2, accounts: [], latest: [], senders: [] }, activity: activityOf(), recentAttachments: [{ email: email('a1', { hasAttachment: true }), fileNames: ['合同.pdf'] }] } satisfies MailOverview,
      props: { draftCount: 2, folders: [inbox(3)] },
      statuses: { c1: ready('482913') },
    }

    it('shows drafts as their own module right after the codes', async () => {
      setup(everything)
      await screen.findByRole('region', { name: '草稿' })
      await waitFor(() => expect(regions()).toEqual(['验证码', '草稿', '收件箱', '近况', '最近的附件']))
    })

    it('gives only the modules that can be hidden a menu, shown to the keyboard', async () => {
      setup({ ...everything, props: { ...everything.props, outbox: [{ id: 'o1', state: 'failed' } as OutboxItem] } })
      const attention = await screen.findByRole('region', { name: '需要处理' })
      expect(within(attention).queryByRole('button', { name: /更多操作/ })).not.toBeInTheDocument()
      const more = within(await screen.findByRole('region', { name: '草稿' })).getByRole('button', { name: '「草稿」的更多操作' })
      // Out of sight until hover or focus, but never out of the tab order.
      expect(more).toHaveClass('opacity-0', 'group-focus-within/module:opacity-100')
      expect(more).not.toHaveAttribute('tabindex', '-1')
    })

    it('hides a module for today, and it returns when its content changes', async () => {
      setup(everything)
      const drafts = await screen.findByRole('region', { name: '草稿' })
      await userEvent.click(within(drafts).getByRole('button', { name: '「草稿」的更多操作' }))
      expect(await screen.findByRole('menuitem', { name: '不再显示' })).toBeInTheDocument()
      await userEvent.click(screen.getByRole('menuitem', { name: '今天先不看' }))
      await waitFor(() => expect(screen.queryByRole('region', { name: '草稿' })).not.toBeInTheDocument())
      expect(screen.getByRole('region', { name: '验证码' })).toBeInTheDocument()
      expect(JSON.parse(localStorage.getItem(SNOOZED_KEY)!).drafts).toEqual({ day: localDay(new Date()), signature: '2' })
      // Undo from the toast.
      await userEvent.click(await screen.findByRole('button', { name: '撤销' }))
      expect(await screen.findByRole('region', { name: '草稿' })).toBeInTheDocument()
      expect(localStorage.getItem(SNOOZED_KEY)).toBeNull()

      // Hidden again; a later visit with the same content keeps it away, new content brings it back.
      localStorage.setItem(SNOOZED_KEY, JSON.stringify({ drafts: { day: localDay(new Date()), signature: '2' } }))
      cleanup()
      setup(everything)
      await screen.findByRole('region', { name: '验证码' })
      expect(screen.queryByRole('region', { name: '草稿' })).not.toBeInTheDocument()
      cleanup()
      setup({ ...everything, props: { ...everything.props, draftCount: 3 } })
      expect(await screen.findByRole('region', { name: '草稿' })).toBeInTheDocument()
    })

    it('forgets a snooze from an earlier day', async () => {
      localStorage.setItem(SNOOZED_KEY, JSON.stringify({ drafts: { day: '2020-01-01', signature: '2' } }))
      setup(everything)
      expect(await screen.findByRole('region', { name: '草稿' })).toBeInTheDocument()
    })

    it('stops showing a module for good, saves it in the settings, and undoes that', async () => {
      const { client } = setup(everything)
      const codes = await screen.findByRole('region', { name: '验证码' })
      await userEvent.click(within(codes).getByRole('button', { name: '「验证码」的更多操作' }))
      await userEvent.click(await screen.findByRole('menuitem', { name: '不再显示' }))
      await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ now: { arrangement: 'auto', order: [], hidden: ['codes'] } }))
      await waitFor(() => expect(screen.queryByRole('region', { name: '验证码' })).not.toBeInTheDocument())
      expect(await screen.findByText('不再显示「验证码」')).toBeInTheDocument()
      expect(screen.getByText('可以在“设置 → 此刻”里恢复')).toBeInTheDocument()
      await userEvent.click(screen.getByRole('button', { name: '撤销' }))
      await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ now: { arrangement: 'auto', order: [], hidden: [] } }))
      expect(await screen.findByRole('region', { name: '验证码' })).toBeInTheDocument()
    })

    it('leaves out the modules the settings hide', async () => {
      setup({ ...everything, now: { hidden: ['codes', 'activity', 'future'] } })
      await screen.findByRole('region', { name: '草稿' })
      await waitFor(() => expect(regions()).toEqual(['草稿', '收件箱', '最近的附件']))
    })

    it('follows a fixed order from the settings, with attention first, and appends the unplaced ones', async () => {
      setup({ ...everything, now: { arrangement: 'fixed', order: ['activity', 'inbox'] }, props: { ...everything.props, outbox: [{ id: 'o1', state: 'failed' } as OutboxItem] } })
      await screen.findByRole('region', { name: '近况' })
      await waitFor(() => expect(regions()).toEqual(['需要处理', '近况', '收件箱', '验证码', '草稿', '最近的附件']))
    })

    it('reports a failed overview once, where its first module would be, and retries', async () => {
      const client = testClient([account])
      client.getMailOverview.mockRejectedValueOnce(new Error('boom'))
      client.getMailOverview.mockResolvedValue({ ...empty, verificationCodes: [overviewCode('c1', '482913')] })
      client.listEmails.mockResolvedValue(page([]))
      client.getVerificationCode.mockImplementation(async (emailId) => ({ emailId, ...ready('482913') }))
      render(
        <TooltipProvider>
          <NowPage client={client} accounts={[account]} folders={[]} syncStates={{}} draftCount={2} outbox={[]} verification={stubVerification()}
            onSelectMailbox={vi.fn()} onOpenUnread={vi.fn()} onOpenContact={vi.fn()} onOpenAccount={vi.fn()} onOpenEmail={vi.fn()} />
        </TooltipProvider>,
      )
      const retry = await screen.findByRole('button', { name: '重试' })
      expect(screen.getAllByRole('alert')).toHaveLength(1)
      expect(screen.getByText('没能读取邮箱概况')).toBeInTheDocument()
      // The page stays: the local modules are still there, below the notice.
      expect(screen.getByRole('heading', { level: 1 })).toBeInTheDocument()
      expect(screen.getByRole('region', { name: '草稿' })).toBeInTheDocument()
      await userEvent.click(retry)
      await waitFor(() => expect(client.getMailOverview).toHaveBeenCalledTimes(2))
      await waitFor(() => expect(screen.queryByText('没能读取邮箱概况')).not.toBeInTheDocument())
    })
  })

  describe('atmosphere and quick actions', () => {
    it('shows the quiet badge when nothing is pending and respects the ambient mode and scene', async () => {
      localStorage.setItem(NOW_AMBIENT_KEY, 'off')
      setup()
      await loaded()
      expect(document.querySelector('[data-slot="now-ambient"]')).toHaveAttribute('data-mode', 'off')
      expect(document.querySelector('[data-slot="now-ambient-svg"]')).toBeNull()
      expect(document.querySelector('[data-slot="now-quiet-badge"]')).toBeInTheDocument()
      cleanup()
      localStorage.setItem(NOW_AMBIENT_KEY, 'full')
      localStorage.setItem(NOW_AMBIENT_SCENE_KEY, 'orbit')
      setup()
      await loaded()
      expect(document.querySelector('[data-slot="now-ambient"]')).toHaveAttribute('data-scene', 'orbit')
      expect(document.querySelector('[data-slot="now-ambient-svg"]')).toHaveAttribute('data-scene', 'orbit')
    })

    it('runs mark-read, star and archive inline on unread rows without opening the message', async () => {
      const target = email('q1', { subject: '审批通知' })
      const { client, onOpenEmail } = setup({ unread: [target], props: { folders: [inbox(1)] } })
      client.archiveEmails.mockResolvedValue({ processedCount: 1, changedCount: 1, undoToken: 'arc-1', skippedIds: [] })
      const section = await screen.findByRole('region', { name: '收件箱' })
      await userEvent.click(within(section).getByRole('button', { name: '加星标' }))
      await waitFor(() => expect(client.setFlags).toHaveBeenCalledWith(['q1'], 'starred', true))
      await userEvent.click(within(section).getByRole('button', { name: '标为已读' }))
      await waitFor(() => expect(client.setFlags).toHaveBeenCalledWith(['q1'], 'read', true))
      await userEvent.click(within(section).getByRole('button', { name: '归档' }))
      await waitFor(() => expect(client.archiveEmails).toHaveBeenCalledWith(['q1']))
      expect(onOpenEmail).not.toHaveBeenCalled()
    })

    it('shows per-account unread pills when multiple accounts exist and opens that account inbox', async () => {
      const second = exampleAccount({ id: 'acc-2', displayName: '个人邮箱' })
      const folder1 = inbox(3)
      const folder2: Folder = { ...inbox(2), id: 'inbox-2', accountId: second.id }
      const { onSelectMailbox } = setup({ accounts: [account, second], props: { folders: [folder1, folder2] } })
      const section = await screen.findByRole('region', { name: '收件箱' })
      await userEvent.click(within(section).getByRole('button', { name: '个人邮箱 收件箱，2 封未读' }))
      expect(onSelectMailbox).toHaveBeenCalledWith({ kind: 'folder', folder: folder2 })
    })
  })
})

/** A controller that never recognizes anything, for tests that do not use codes. */
function stubVerification(): NowPageProps['verification'] {
  return { status: () => undefined, busy: () => false, copied: () => false, copy: vi.fn(), requestVisible: () => undefined }
}
