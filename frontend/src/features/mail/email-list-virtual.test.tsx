import { cleanup, render, screen, within } from '@/test/render'
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import type { EmailSummary, PreviewLines, WeekStartsOn } from '@/generated/accounts'
import { testViewport } from '@/test/viewport'
import { EmailList } from './email-list'

function email(index: number, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    senderGroup: null,
    id: `m${index}`, accountId: 'work', folderId: 'inbox', subject: `主题 ${index}`,
    from: { name: 'GitHub', address: 'notifications@github.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

const many = (count: number) => Array.from({ length: count }, (_, index) => email(index))

type Options = {
  hasMore?: boolean
  loadingMore?: boolean
  loadMoreError?: string | null
  selectedId?: string | null
  previewLines?: PreviewLines
  groupByDate?: boolean
  weekStartsOn?: WeekStartsOn
}

function renderList(emails: EmailSummary[], options: Options = {}) {
  const onLoadMore = vi.fn()
  const onFilterChange = vi.fn()
  const onSelect = vi.fn()
  const list = (items: EmailSummary[], { hasMore = false, loadingMore = false, loadMoreError = null, selectedId = null, previewLines = 'one', groupByDate, weekStartsOn }: Options) => (
    <EmailList
      title="全部收件箱" emails={items} accounts={null} loading={false}
      syncStatus={{ phase: 'idle', failure: null, windowCapped: false }}
      hasMore={hasMore} loadingMore={loadingMore} refreshing={false} stale={false} loadMoreError={loadMoreError} error={null}
      filter="all" search="" selectedId={selectedId} previewLines={previewLines}
      groupByDate={groupByDate} weekStartsOn={weekStartsOn}
      onFilterChange={onFilterChange} onSelect={onSelect} onLoadMore={onLoadMore}
    />
  )
  const { rerender } = render(list(emails, options))
  return {
    onLoadMore,
    rerenderList: (items: EmailSummary[], next: Options = {}) => rerender(list(items, { ...options, ...next })),
  }
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
    const { onLoadMore } = renderList(many(500), { hasMore: true })
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
    const { onLoadMore } = renderList(many(500), { hasMore: true, ...options })
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

describe('date headings in the virtualized list', () => {
  const monday = new Date(2026, 10, 23, 12)
  const receivedOn = (index: number, day: number, hour: number) => email(index, {
    receivedAt: new Date(2026, 10, day, hour).toISOString(),
  })
  const headingTexts = () => Array.from(document.querySelectorAll('[data-slot="email-date-heading"]'), (heading) => heading.textContent)

  it('updates the groups for a different week start and can return to a flat list', () => {
    vi.useFakeTimers({ now: monday, toFake: ['Date'] })
    try {
      const emails = [receivedOn(0, 23, 9), receivedOn(1, 22, 9), receivedOn(2, 21, 9)]
      const { rerenderList } = renderList(emails, { selectedId: 'm2' })
      expect(headingTexts()).toEqual(['今天', '昨天', '上周'])

      const rowButton = within(rows()[2]).getByRole('button')
      act(() => { rowButton.focus() })
      rerenderList(emails, { weekStartsOn: 'saturday' })
      expect(headingTexts()).toEqual(['今天', '昨天', '本周'])
      expect(rowButton).toHaveAccessibleDescription('本周')
      expect(rowButton).toHaveFocus()
      expect(rows()[2]).toHaveAttribute('aria-current', 'true')
      expect(rows()[1].querySelector('time')).toHaveTextContent('09:00')

      rerenderList(emails, { groupByDate: false })
      expect(headingTexts()).toEqual([])
      expect(rowButton).not.toHaveAttribute('aria-description')
      expect(rowButton).toHaveFocus()
      expect(rows()[1].querySelector('time')).toHaveTextContent('昨天')
    } finally {
      vi.useRealTimers()
    }
  })

  it('extends the current group when the next page starts in it and numbers only emails', () => {
    vi.useFakeTimers({ now: monday, toFake: ['Date'] })
    try {
      const firstPage = [receivedOn(0, 23, 9), receivedOn(1, 22, 9)]
      const { rerenderList } = renderList(firstPage, { hasMore: true })
      expect(headingTexts()).toEqual(['今天', '昨天'])

      const nextPage = [receivedOn(2, 22, 8), receivedOn(3, 21, 9)]
      rerenderList([...firstPage, ...nextPage], { hasMore: false })
      expect(headingTexts()).toEqual(['今天', '昨天', '上周'])
      expect(rows()).toHaveLength(4)
      for (const [index, row] of rows().entries()) {
        expect(row).toHaveAttribute('aria-setsize', '4')
        expect(row).toHaveAttribute('aria-posinset', String(index + 1))
      }
    } finally {
      vi.useRealTimers()
    }
  })

  it('moves the current groups forward at local midnight', () => {
    vi.useFakeTimers({ now: new Date(2026, 10, 23, 23, 59, 59, 500) })
    try {
      renderList([receivedOn(0, 23, 9), receivedOn(1, 22, 9)])
      expect(headingTexts()).toEqual(['今天', '昨天'])

      act(() => { vi.advanceTimersByTime(500) })
      expect(headingTexts()).toEqual(['昨天', '上周'])
    } finally {
      vi.useRealTimers()
    }
  })
})
