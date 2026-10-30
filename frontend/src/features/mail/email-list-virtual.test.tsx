import { cleanup, render, screen, within } from '@/test/render'
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import type { EmailSummary, PreviewLines } from '@/generated/accounts'
import { testViewport } from '@/test/viewport'
import { EmailList } from './email-list'

function email(index: number, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id: `m${index}`, accountId: 'work', folderId: 'inbox', subject: `主题 ${index}`,
    from: { name: 'GitHub', address: 'notifications@github.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, ...overrides,
  }
}

const many = (count: number) => Array.from({ length: count }, (_, index) => email(index))

type Options = { hasMore?: boolean; loadingMore?: boolean; loadMoreError?: string | null; selectedId?: string | null; previewLines?: PreviewLines }

function renderList(emails: EmailSummary[], { hasMore = false, loadingMore = false, loadMoreError = null, selectedId = null, previewLines = 'one' }: Options = {}) {
  const onLoadMore = vi.fn()
  render(
    <EmailList
      title="全部收件箱" emails={emails} accounts={null} loading={false}
      syncStatus={{ phase: 'idle', failure: null, windowCapped: false }}
      hasMore={hasMore} loadingMore={loadingMore} refreshing={false} stale={false} loadMoreError={loadMoreError} error={null}
      filter="all" search="" selectedId={selectedId} previewLines={previewLines}
      onFilterChange={vi.fn()} onSelect={vi.fn()} onLoadMore={onLoadMore}
    />,
  )
  return onLoadMore
}

const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).queryAllByRole('listitem')
const viewport = () => document.querySelector<HTMLElement>('[data-slot="scroll-area-viewport"]')!

async function scrollTo(top: number) {
  await act(async () => {
    viewport().scrollTop = top
    viewport().dispatchEvent(new Event('scroll'))
  })
}

describe('virtualized mail list', () => {
  it('renders only a window of a long list', () => {
    testViewport.height = 600
    renderList(many(500))
    expect(rows().length).toBeGreaterThan(0)
    expect(rows().length).toBeLessThan(40)
    expect(rows()[0]).toHaveTextContent('主题 0')
  })

  it('numbers every row against the whole list', () => {
    testViewport.height = 600
    renderList(many(500))
    for (const [index, row] of rows().entries()) {
      expect(row).toHaveAttribute('aria-setsize', '500')
      expect(row).toHaveAttribute('aria-posinset', String(index + 1))
    }
  })

  it('keeps the selected row mounted after scrolling far away', async () => {
    testViewport.height = 600
    renderList(many(500), { selectedId: 'm3' })
    await scrollTo(30000)
    expect(rows().some((row) => row.textContent?.includes('主题 3'))).toBe(true)
    expect(rows().some((row) => row.textContent?.includes('主题 0'))).toBe(false)
    expect(rows().length).toBeLessThan(40)
  })

  it('asks for the next page near the end, once the list has more', async () => {
    testViewport.height = 600
    const onLoadMore = renderList(many(500), { hasMore: true })
    expect(onLoadMore).not.toHaveBeenCalled()
    await scrollTo(1_000_000)
    expect(onLoadMore).toHaveBeenCalled()
  })

  it.each([
    ['a load is already running', { loadingMore: true }],
    ['the last load failed', { loadMoreError: '操作未完成，请重试。' }],
    ['there is nothing more to load', { hasMore: false }],
  ] as const)('does not auto-load when %s', async (_, options) => {
    testViewport.height = 600
    const onLoadMore = renderList(many(500), { hasMore: true, ...options })
    await scrollTo(1_000_000)
    expect(onLoadMore).not.toHaveBeenCalled()
    cleanup()
  })

  it('reserves the same preview height for rows with and without a preview', () => {
    renderList([email(0, { preview: '这是预览' }), email(1)], { previewLines: 'two' })
    const slots = rows().map((row) => row.querySelector('p')!)
    expect(slots[0]).toHaveTextContent('这是预览')
    expect(slots[1]).toBeEmptyDOMElement()
    for (const slot of slots) expect(slot).toHaveClass('h-8', 'overflow-hidden', 'line-clamp-2')
  })

  it('reserves no preview slot when previews are off', () => {
    renderList([email(0, { preview: '这是预览' })], { previewLines: 'none' })
    expect(rows()[0].querySelector('p')).toBeNull()
  })
})
