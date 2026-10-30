import { render, screen, within } from '@/test/render'
import { describe, expect, it, vi } from 'vitest'
import type { EmailSummary } from '@/generated/accounts'
import { errorText } from '@/i18n/errors'
import { MailxClientError } from '@/lib/mailx-client'
import { EmailList } from './email-list'

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id, accountId: 'work', folderId: 'inbox', subject: `主题 ${id}`,
    from: { name: 'GitHub', address: 'notifications@github.com' }, preview: `预览 ${id}`, receivedAt: '2026-10-20T00:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, ...overrides,
  }
}

function renderList(emails: EmailSummary[], search: string, indexedBodies = 0, windowCapped = false) {
  render(
    <EmailList
      title="全部收件箱" sentView={false} emails={emails} accounts={null} loading={false}
      syncStatus={{ phase: 'idle', failure: null, windowCapped }}
      hasMore={false} loadingMore={false} refreshing={false} stale={false} loadMoreError={null} error={null}
      filter="all" search={search} indexedBodies={indexedBodies} selectedId={null} previewLines="two"
      onFilterChange={vi.fn()} onSelect={vi.fn()} onLoadMore={vi.fn()}
    />,
  )
  return within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
}

describe('search snippets in the list', () => {
  it('shows the snippet in place of the preview, marking hits as text', () => {
    const [row] = renderList([email('a', {
      matchSnippet: [{ text: '…合同', hit: false }, { text: '<img src=x onerror=alert(1)>', hit: true }, { text: '已签', hit: false }],
    })], '合同')
    expect(within(row).queryByText('预览 a')).not.toBeInTheDocument()
    const mark = row.querySelector('mark')!
    expect(mark.textContent).toBe('<img src=x onerror=alert(1)>')
    expect(mark).not.toHaveClass('border')
    expect(row.querySelector('img')).toBeNull()
    expect(row.textContent).toContain('…合同<img src=x onerror=alert(1)>已签')
  })

  it('keeps the preview when there is no snippet', () => {
    const [row] = renderList([email('a')], '主题')
    expect(within(row).getByText('预览 a')).toBeInTheDocument()
    expect(row.querySelector('mark')).toBeNull()
  })
})

describe('search coverage hint', () => {
  it('is shown only while searching, with the indexed count', () => {
    renderList([email('a')], '合同', 12)
    expect(screen.getByText('已在本地索引 12 封邮件的正文中搜索')).toBeInTheDocument()
  })

  it('is absent without search text', () => {
    renderList([email('a')], '', 12)
    expect(screen.queryByText(/已在本地索引/)).not.toBeInTheDocument()
  })

  it('shares one line with the sync window notice instead of stacking two', () => {
    renderList([email('a')], '合同', 3, true)
    expect(screen.getAllByText(/同步覆盖受限/)).toHaveLength(1)
    expect(screen.getByText(/已在本地索引 3 封邮件的正文中搜索 同步覆盖受限/)).toBeInTheDocument()
  })
})

describe('search validation error', () => {
  it('names the limits instead of a generic error', () => {
    const error = new MailxClientError({ code: 'validation', message: 'x', field: 'text' })
    expect(errorText(error)).toBe('搜索内容过长或词太多：最多 16 个词、256 个字符。')
  })
})
