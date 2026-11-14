import { useCallback, useState } from 'react'
import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Account, AccountSyncState, EmailPage, EmailSummary, Folder, MailOverview, OutboxItem, OverviewActivity, OverviewCode, VerificationCodeStatus } from '@/generated/accounts'
import { exampleAccount, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import { TooltipProvider } from '@/components/ui/tooltip'
import type { MailxClient } from '@/lib/mailx-client'
import { useVerificationCodes, type VerificationTarget } from '@/features/mail/use-verification-codes'
import { localDay } from './greeting'
import { ENTRANCE_KEY, LAST_VISIT_KEY } from './visit'
import { dayStarts } from './activity'
import { NowPage, type NowPageProps } from './now-page'

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
  return { items, nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }
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

const empty: MailOverview = { newMail: { total: 0, accounts: [], latest: [] }, starredUnread: 0, verificationCodes: [], recentAttachments: [], generatedAt: new Date().toISOString() }

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

function setup({ overview = empty, unread = [], accounts = [account], props = {}, statuses = {} }: {
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
    await userEvent.click(within(section).getByRole('button', { name: '2 份草稿未完成' }))
    expect(onSelectMailbox).toHaveBeenLastCalledWith({ kind: 'drafts' })
  })

  describe('inbox', () => {
    it('opens new mail, unread with the Unread filter, and starred unread', async () => {
      const { onSelectMailbox, onOpenUnread } = setup({
        overview: { ...empty, newMail: { total: 3, accounts: [{ accountId: account.id, count: 3 }], latest: [] }, starredUnread: 2 },
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
      setup({ props: { syncStates: synced({ failure: { code: 'network', message: '', detail: null, willRetry: true } }) } })
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

  it('hides a code that is already older than fifteen minutes', async () => {
    setup({
      overview: { ...empty, starredUnread: 1, verificationCodes: [overviewCode('c1', '111111', { receivedAt: new Date(Date.now() - 16 * 60_000).toISOString() })] },
    })
    await screen.findByRole('region', { name: '收件箱' })
    expect(screen.queryByRole('region', { name: '验证码' })).not.toBeInTheDocument()
  })

  it('lists recent attachments by file name, or a generic label', async () => {
    const named = email('a1', { hasAttachment: true })
    const unnamed = email('a2', { hasAttachment: true })
    const { onOpenEmail } = setup({ overview: { ...empty, recentAttachments: [{ email: named, fileNames: ['合同.pdf', '附录.docx'] }, { email: unnamed, fileNames: [] }] } })
    const section = await screen.findByRole('region', { name: '最近的附件' })
    expect(within(section).getByText('合同.pdf')).toBeInTheDocument()
    expect(within(section).getByText('等 1 个文件')).toBeInTheDocument()
    await userEvent.click(within(section).getByRole('button', { name: /^附件/ }))
    expect(onOpenEmail).toHaveBeenCalledWith(unnamed)
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

  it('renders at once on a later visit the same day', async () => {
    localStorage.setItem(ENTRANCE_KEY, localDay(new Date()))
    setup({ overview: { ...empty, starredUnread: 2, activity: activityOf() } })
    await loaded()
    expect(screen.getByRole('heading', { level: 1 }).parentElement).not.toHaveClass('animate-in')
    expect(document.querySelector('[data-slot="activity-bar"]')).not.toHaveClass('animate-now-bar-grow')
  })

  it('skips the rise, stagger, count-up and growing bars with reduced motion', async () => {
    vi.spyOn(window, 'matchMedia').mockImplementation((query: string) => ({
      matches: query.includes('reduce'), media: query, onchange: null,
      addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
    }))
    const raf = vi.spyOn(window, 'requestAnimationFrame')
    setup({ overview: { ...empty, newMail: { total: 12, accounts: [], latest: [] }, activity: activityOf() } })
    const section = await screen.findByRole('region', { name: '收件箱' })
    expect(section).not.toHaveClass('animate-in')
    expect(screen.getByRole('heading', { level: 1 }).parentElement).not.toHaveClass('animate-in')
    expect(within(section).getByRole('button', { name: '新邮件 12' })).toHaveTextContent('12')
    expect(document.querySelector('[data-slot="activity-bar"]')).not.toHaveClass('animate-now-bar-grow')
    expect(raf).not.toHaveBeenCalled()
  })
})

/** A controller that never recognizes anything, for tests that do not use codes. */
function stubVerification(): NowPageProps['verification'] {
  return { status: () => undefined, busy: () => false, copied: () => false, copy: vi.fn(), requestVisible: () => undefined }
}
