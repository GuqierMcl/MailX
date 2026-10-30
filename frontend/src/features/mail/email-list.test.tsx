import { cleanup, render, screen, within } from '@/test/render'
import { describe, expect, it, vi } from 'vitest'
import { exampleAccount } from '@/features/accounts/test-fixtures'
import type { Account, EmailSummary, PreviewLines } from '@/generated/accounts'
import { EmailList } from './email-list'
import { formatListTime } from './format'

const work = exampleAccount({ id: 'work', displayName: '工作邮箱', emailAddress: 'lin.wei@example.com' })
const home = exampleAccount({ id: 'home', displayName: '个人邮箱', emailAddress: 'linwei.home@example.org' })

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id, accountId: work.id, folderId: 'inbox', subject: `主题 ${id}`,
    from: { name: 'GitHub', address: 'notifications@github.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, to: { addresses: [], total: 0 }, localOnly: false, ...overrides,
  }
}

function renderList(emails: EmailSummary[], accounts: Account[] | null, previewLines: PreviewLines = 'one', sentView = false) {
  render(
    <EmailList
      title="全部收件箱" sentView={sentView} emails={emails} accounts={accounts} loading={false}
      syncStatus={{ phase: 'idle', failure: null, windowCapped: false }}
      hasMore={false} loadingMore={false} refreshing={false} stale={false} loadMoreError={null} error={null}
      filter="all" search="" selectedId={null} previewLines={previewLines}
      onFilterChange={vi.fn()} onSelect={vi.fn()} onLoadMore={vi.fn()}
    />,
  )
  return within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
}

describe('which mailbox a message belongs to', () => {
  it('sits after the sender in a muted, small line, not under the card', () => {
    const [row] = renderList([email('a')], [work, home])
    const senderLine = within(row).getByText('GitHub').parentElement!
    // "GitHub | 工作邮箱" on one line, the mailbox name muted and small.
    const owner = within(senderLine).getByText('工作邮箱')
    expect(owner).toHaveClass('text-muted-foreground', 'text-xs')
    expect(senderLine.textContent).toMatch(/GitHub\s*\|\s*工作邮箱/)
    // The mailbox is named, not addressed, and no badge sits below the preview any more.
    expect(within(row).queryByText('lin.wei@example.com')).not.toBeInTheDocument()
    expect(row.querySelector('[data-slot="badge"]')).toBeNull()
  })

  it('names the right account for each message', () => {
    const rows = renderList([email('a'), email('b', { accountId: home.id })], [work, home])
    expect(within(rows[0]).getByText('工作邮箱')).toBeInTheDocument()
    expect(within(rows[1]).getByText('个人邮箱')).toBeInTheDocument()
  })

  it('is not shown in a single-account view', () => {
    const [row] = renderList([email('a')], null)
    expect(within(row).queryByText('工作邮箱')).not.toBeInTheDocument()
    expect(row.textContent).not.toContain('|')
  })

  it('keeps the time and truncates the sender and mailbox name instead of pushing it out', () => {
    const [row] = renderList([email('a', { from: { name: 'A very long sender name that cannot fit', address: 'x@y.z' } })], [work])
    // The time is always there and never shrinks; only the names give way.
    const time = within(row).getByText(formatListTime('2026-10-20T00:00:00Z'))
    expect(time).toHaveClass('shrink-0')
    expect(within(row).getByText(/A very long sender/)).toHaveClass('truncate')
    expect(within(row).getByText('工作邮箱')).toHaveClass('truncate')
  })
})

describe('starred messages', () => {
  it('show a yellow filled star before the subject, not after it', () => {
    const [row] = renderList([email('a', { isStarred: true, hasAttachment: true })], null)
    const star = within(row).getByLabelText('已加星标')
    const subject = within(row).getByText('主题 a')
    expect(star).toHaveClass('text-amber-500')
    expect(star.getAttribute('fill')).toBe('currentColor')
    // Document order: star, then subject, then the attachment icon.
    const attachment = within(row).getByLabelText('有附件')
    expect(star.compareDocumentPosition(subject) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(subject.compareDocumentPosition(attachment) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('leave unstarred messages without any star', () => {
    const [row] = renderList([email('a')], null)
    expect(within(row).queryByLabelText('已加星标')).not.toBeInTheDocument()
  })
})

describe('body preview lines', () => {
  const long = email('a', { preview: '第一行内容。'.repeat(40) })

  it('shows one line by default and follows the setting', () => {
    for (const [lines, clamp] of [['one', 'line-clamp-1'], ['two', 'line-clamp-2'], ['three', 'line-clamp-3']] as const) {
      const [row] = renderList([long], null, lines)
      expect(within(row).getByText(long.preview)).toHaveClass(clamp)
      cleanup()
    }
  })

  it('hides the preview entirely when set to none', () => {
    const [row] = renderList([long], null, 'none')
    expect(within(row).queryByText(long.preview)).not.toBeInTheDocument()
    expect(within(row).getByText(long.subject)).toBeInTheDocument()
  })
})

describe('rows in a sent view', () => {
  const addr = (name: string | null, address: string) => ({ name, address })

  it('name the first recipient and the total instead of the sender', () => {
    const [row] = renderList([email('a', { to: { addresses: [addr('陈宇', 'chen@example.com'), addr(null, 'zhao@example.com')], total: 5 } })], null, 'one', true)
    expect(within(row).getByText('致：陈宇 等 5 人')).toBeInTheDocument()
    expect(within(row).queryByText('GitHub')).not.toBeInTheDocument()
  })

  it('name a single recipient without a count', () => {
    const [row] = renderList([email('a', { to: { addresses: [addr(null, 'chen@example.com')], total: 1 } })], null, 'one', true)
    expect(within(row).getByText('致：chen@example.com')).toBeInTheDocument()
  })

  it('leave other mailboxes showing the sender', () => {
    const [row] = renderList([email('a', { to: { addresses: [addr('陈宇', 'chen@example.com')], total: 1 } })], null)
    expect(within(row).getByText('GitHub')).toBeInTheDocument()
    expect(within(row).queryByText(/致：/)).not.toBeInTheDocument()
  })

  it('mark a local-only copy with a tooltip, and nothing else', async () => {
    const rows = renderList([email('local-sent:1', { localOnly: true, to: { addresses: [addr(null, 'a@b.c')], total: 1 } }), email('b')], null, 'one', true)
    const marker = within(rows[0]).getByRole('img', { name: '仅在本机' })
    expect(within(rows[1]).queryByRole('img', { name: '仅在本机' })).not.toBeInTheDocument()
    const { default: userEvent } = await import('@testing-library/user-event')
    await userEvent.hover(marker)
    expect(await screen.findByText('仅在本机，服务器上暂未找到副本', { selector: '[data-slot="tooltip-content"]' })).toBeInTheDocument()
  })
})
