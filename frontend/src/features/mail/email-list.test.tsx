import { render, screen, within } from '@/test/render'
import { describe, expect, it, vi } from 'vitest'
import { exampleAccount } from '@/features/accounts/test-fixtures'
import type { Account, EmailSummary } from '@/generated/accounts'
import { EmailList } from './email-list'
import { formatListTime } from './format'

const work = exampleAccount({ id: 'work', displayName: '工作邮箱', emailAddress: 'lin.wei@example.com' })
const home = exampleAccount({ id: 'home', displayName: '个人邮箱', emailAddress: 'linwei.home@example.org' })

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id, accountId: work.id, folderId: 'inbox', subject: `主题 ${id}`,
    from: { name: 'GitHub', address: 'notifications@github.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', pendingChanges: false, ...overrides,
  }
}

function renderList(emails: EmailSummary[], accounts: Account[] | null) {
  render(
    <EmailList
      title="全部收件箱" emails={emails} accounts={accounts} loading={false}
      syncStatus={{ phase: 'idle', failure: null, windowCapped: false }}
      hasMore={false} loadingMore={false} refreshing={false} stale={false} loadMoreError={null} error={null}
      filter="all" search="" selectedId={null}
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
