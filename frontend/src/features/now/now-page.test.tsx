import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Account, EmailSummary, MailOverview, OutboxItem } from '@/generated/accounts'
import { exampleAccount, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import { TooltipProvider } from '@/components/ui/tooltip'
import { localDay } from './greeting'
import { ENTRANCE_KEY, LAST_VISIT_KEY } from './visit'
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

const empty: MailOverview = { newMail: { total: 0, accounts: [], latest: [] }, starredUnread: 0, verificationCodes: [], recentAttachments: [], generatedAt: new Date().toISOString() }

function setup({ overview = empty, accounts = [account], props = {} }: { overview?: MailOverview; accounts?: Account[]; props?: Partial<NowPageProps> } = {}) {
  const client = testClient(accounts)
  client.getMailOverview.mockResolvedValue(overview)
  const handlers = { onSelectMailbox: vi.fn(), onOpenAccount: vi.fn(), onOpenEmail: vi.fn() }
  render(
    <TooltipProvider>
      <NowPage client={client} accounts={accounts} folders={[]} syncStates={{}} draftCount={0} outbox={[]} {...handlers} {...props} />
      <Toaster />
    </TooltipProvider>,
  )
  return { client, ...handlers }
}

/** Waits for the overview: the summary line appears with it. */
const loaded = async () => {
  const heading = await screen.findByRole('heading', { level: 1 })
  await waitFor(() => expect(heading.parentElement!.querySelectorAll('p')[1]).not.toBeEmptyDOMElement())
}

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
    // The first visit counts from the start of today.
    const start = new Date()
    start.setHours(0, 0, 0, 0)
    expect(client.getMailOverview).toHaveBeenCalledWith({ since: start.toISOString() })
  })

  it('greets without a name when the sending account has none', async () => {
    setup({ accounts: [exampleAccount({ senderName: null })] })
    await loaded()
    expect(screen.getByRole('heading', { level: 1 }).textContent).not.toMatch(/，/)
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

  it('shows new mail since the last visit and opens a message', async () => {
    const since = new Date(Date.now() - 3_600_000).toISOString()
    localStorage.setItem(LAST_VISIT_KEY, since)
    const latest = email('m1', { subject: '季度报告' })
    const { client, onOpenEmail } = setup({ overview: { ...empty, newMail: { total: 3, accounts: [{ accountId: account.id, count: 3 }], latest: [latest] } } })
    const section = await screen.findByRole('region', { name: '自上次查看后的新邮件' })
    expect(client.getMailOverview).toHaveBeenCalledWith({ since })
    expect(within(section).getByText('3 封新邮件')).toBeInTheDocument()
    await userEvent.click(within(section).getByRole('button', { name: /季度报告/ }))
    expect(onOpenEmail).toHaveBeenCalledWith(latest)
  })

  it('opens the unread and starred views', async () => {
    const { onSelectMailbox } = setup({
      overview: { ...empty, starredUnread: 2 },
      props: { folders: [{ id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox', selectable: true, countsKnown: true, unreadCount: 7, totalCount: 9, archivedCount: 0, archivedUnreadCount: 0 }] },
    })
    const section = await screen.findByRole('region', { name: '未读与星标' })
    await userEvent.click(within(section).getByRole('button', { name: '收件箱未读 7' }))
    expect(onSelectMailbox).toHaveBeenLastCalledWith({ kind: 'unified', role: 'inbox' })
    await userEvent.click(within(section).getByRole('button', { name: '星标未读 2' }))
    expect(onSelectMailbox).toHaveBeenLastCalledWith({ kind: 'starred' })
  })

  it('copies a verification code and opens its message from the sender', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    const { onOpenEmail } = setup({
      overview: { ...empty, verificationCodes: [{ emailId: 'c1', accountId: account.id, folderId: 'inbox', from: { name: '银行', address: 'bank@example.com' }, code: '482913', receivedAt: new Date(Date.now() - 60_000).toISOString() }] },
    })
    const section = await screen.findByRole('region', { name: '验证码' })
    expect(within(section).getByText('验证码 482913')).toBeInTheDocument()
    await userEvent.click(within(section).getByRole('button', { name: '复制验证码 482913' }))
    expect(writeText).toHaveBeenCalledWith('482913')
    expect(await screen.findByText('已复制验证码')).toBeInTheDocument()
    await userEvent.click(within(section).getByRole('button', { name: '银行' }))
    expect(onOpenEmail).toHaveBeenCalledWith({ id: 'c1', accountId: account.id })
  })

  it('hides a code that is already older than fifteen minutes', async () => {
    setup({
      overview: { ...empty, starredUnread: 1, verificationCodes: [{ emailId: 'c1', accountId: account.id, folderId: 'inbox', from: { name: null, address: 'bank@example.com' }, code: '111111', receivedAt: new Date(Date.now() - 16 * 60_000).toISOString() }] },
    })
    await screen.findByRole('region', { name: '未读与星标' })
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

  it('plays the entrance on the first visit of the day only', async () => {
    setup({ overview: { ...empty, starredUnread: 2 } })
    await loaded()
    expect(screen.getByRole('heading', { level: 1 }).parentElement).toHaveClass('animate-in')
    expect(localStorage.getItem(ENTRANCE_KEY)).toBe(localDay(new Date()))
  })

  it('renders at once on a later visit the same day', async () => {
    localStorage.setItem(ENTRANCE_KEY, localDay(new Date()))
    setup({ overview: { ...empty, starredUnread: 2 } })
    await loaded()
    expect(screen.getByRole('heading', { level: 1 }).parentElement).not.toHaveClass('animate-in')
  })

  it('skips the rise, stagger and count-up with reduced motion', async () => {
    vi.spyOn(window, 'matchMedia').mockImplementation((query: string) => ({
      matches: query.includes('reduce'), media: query, onchange: null,
      addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
    }))
    const raf = vi.spyOn(window, 'requestAnimationFrame')
    setup({ overview: { ...empty, newMail: { total: 12, accounts: [], latest: [] } } })
    const section = await screen.findByRole('region', { name: '自上次查看后的新邮件' })
    expect(section).not.toHaveClass('animate-in')
    expect(screen.getByRole('heading', { level: 1 }).parentElement).not.toHaveClass('animate-in')
    expect(within(section).getByText('12')).toBeInTheDocument()
    expect(raf).not.toHaveBeenCalled()
  })
})
