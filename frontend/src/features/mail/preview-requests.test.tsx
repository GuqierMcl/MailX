import { act, render, renderHook, screen, within } from '@/test/render'
import { QueryClientProvider, type InfiniteData } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import type { EmailPage, EmailPreviews, EmailSummary, PreviewLines } from '@/generated/accounts'
import { createQueryClient, mailKeys } from '@/lib/query-client'
import { testViewport } from '@/test/viewport'
import { EmailList } from './email-list'
import { GAVE_UP_MS, giveUpPreview, resetGaveUp } from './preview-gave-up'
import { useEmailPreviews } from './use-email-previews'

function email(index: number, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id: `m${index}`, accountId: 'work', folderId: 'inbox', subject: `主题 ${index}`,
    from: { name: 'GitHub', address: 'notifications@github.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'missing', previewState: 'missing', pendingChanges: false, ...overrides,
  }
}

const many = (count: number) => Array.from({ length: count }, (_, index) => email(index))

function list(emails: EmailSummary[], previewLines: PreviewLines, onRequestPreviews: (ids: string[]) => void) {
  return (
    <EmailList
      title="全部收件箱" emails={emails} accounts={null} loading={false}
      syncStatus={{ phase: 'idle', failure: null, windowCapped: false }}
      hasMore={false} loadingMore={false} refreshing={false} stale={false} loadMoreError={null} error={null}
      filter="all" search="" selectedId={null} previewLines={previewLines} onRequestPreviews={onRequestPreviews}
      onFilterChange={vi.fn()} onSelect={vi.fn()} onLoadMore={vi.fn()}
    />
  )
}

const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).queryAllByRole('listitem')
const previewSlot = (row: HTMLElement) => row.querySelector('p')!

async function advance(ms: number) {
  await act(async () => { await vi.advanceTimersByTimeAsync(ms) })
}

beforeEach(() => {
  vi.useFakeTimers()
  resetGaveUp()
  testViewport.height = 600
})
afterEach(() => {
  vi.useRealTimers()
  testViewport.height = 10000
})

describe('preview requests', () => {
  it('asks once, sorted, for the rendered rows still missing a preview', async () => {
    const request = vi.fn()
    const emails = many(500)
    emails[1] = email(1, { previewState: 'ready', preview: '已有' })
    emails[2] = email(2, { bodyState: 'ready' })
    render(list(emails, 'one', request))
    expect(request).not.toHaveBeenCalled()
    await advance(300)
    expect(request).toHaveBeenCalledTimes(1)
    const ids: string[] = request.mock.calls[0][0]
    expect(ids).toEqual([...ids].sort())
    expect(new Set(ids).size).toBe(ids.length)
    expect(ids).toContain('m0')
    expect(ids).not.toContain('m1')
    expect(ids).not.toContain('m2')
    expect(ids.length).toBeLessThan(rows().length + 1)
    expect(ids).not.toContain('m400')
  })

  it('sends nothing while previews are off, and the visible rows once they are turned on', async () => {
    const request = vi.fn()
    const emails = many(20)
    const { rerender } = render(list(emails, 'none', request))
    await advance(500)
    expect(request).not.toHaveBeenCalled()
    rerender(list(emails, 'one', request))
    await advance(300)
    expect(request).toHaveBeenCalledTimes(1)
    expect(request.mock.calls[0][0]).toEqual(expect.arrayContaining(['m0', 'm1']))
    expect(request.mock.calls[0][0].length).toBeLessThanOrEqual(20)
  })

  it('cancels with one empty request when previews are turned off', async () => {
    const request = vi.fn()
    const emails = many(5)
    const { rerender } = render(list(emails, 'one', request))
    await advance(300)
    rerender(list(emails, 'none', request))
    await advance(300)
    expect(request).toHaveBeenCalledTimes(2)
    expect(request.mock.calls[1][0]).toEqual([])
    await advance(1000)
    expect(request).toHaveBeenCalledTimes(2)
  })

  it('sends the empty set once when nothing is wanted any more', async () => {
    const request = vi.fn()
    const { rerender } = render(list(many(3), 'one', request))
    await advance(300)
    rerender(list(many(3).map((item) => ({ ...item, previewState: 'ready' as const })), 'one', request))
    await advance(300)
    expect(request.mock.calls.map(([ids]) => (ids as string[]).length)).toEqual([3, 0])
    await advance(1000)
    expect(request).toHaveBeenCalledTimes(2)
  })

  it('does not repeat an identical wanted set', async () => {
    const request = vi.fn()
    const emails = many(3)
    const { rerender } = render(list(emails, 'one', request))
    await advance(300)
    rerender(list([...emails], 'one', request))
    await advance(300)
    expect(request).toHaveBeenCalledTimes(1)
  })
})

describe('preview display', () => {
  it('says loading on the first line of a missing row and keeps the row height class', () => {
    const request = vi.fn()
    const emails = many(2)
    const { rerender } = render(list(emails, 'two', request))
    const before = previewSlot(rows()[0]).className
    expect(previewSlot(rows()[0])).toHaveTextContent('（加载中）')
    rerender(list([email(0, { previewState: 'ready', preview: '你好' }), emails[1]], 'two', request))
    expect(previewSlot(rows()[0])).toHaveTextContent('你好')
    expect(previewSlot(rows()[0]).className).toBe(before)
    expect(previewSlot(rows()[0]).className).toContain('h-8')
  })

  it('leaves failed rows, empty ready rows and rows with a body ready', () => {
    render(list([
      email(0, { previewState: 'failed' }),
      email(1, { previewState: 'ready', preview: '' }),
      email(2, { bodyState: 'ready', preview: '正文预览' }),
    ], 'one', vi.fn()))
    expect(previewSlot(rows()[0])).toBeEmptyDOMElement()
    expect(previewSlot(rows()[1])).toBeEmptyDOMElement()
    expect(previewSlot(rows()[2])).toHaveTextContent('正文预览')
  })

  it('goes blank after an attempt ended without a result and is not asked again for 60 s', async () => {
    const request = vi.fn()
    const emails = many(2)
    render(list(emails, 'one', request))
    await advance(300)
    expect(request).toHaveBeenCalledTimes(1)
    act(() => { giveUpPreview('m0'); giveUpPreview('m1') })
    expect(previewSlot(rows()[0])).toBeEmptyDOMElement()
    await advance(300)
    expect(request.mock.calls.at(-1)![0]).toEqual([])
    const calls = request.mock.calls.length
    await advance(GAVE_UP_MS - 2000)
    expect(request).toHaveBeenCalledTimes(calls)
    await advance(3000)
    await advance(300)
    expect(request.mock.calls.at(-1)![0]).toEqual(['m0', 'm1'])
    expect(previewSlot(rows()[0])).toHaveTextContent('（加载中）')
  })
})

describe('preview results', () => {
  const page = (items: EmailSummary[]): EmailPage => ({
    items, nextCursor: null,
    coverage: { headersComplete: true, bodiesComplete: true, syncedAfter: null, syncedBefore: null, windowCapped: false },
  })

  function setup() {
    const client = testClient()
    let emit: (previews: EmailPreviews) => void = () => undefined
    client.onEmailPreviews.mockImplementation((listener) => { emit = listener; return () => undefined })
    const queryClient = createQueryClient()
    const key = [...mailKeys.emailLists(), 'sources', { folderId: 'inbox' }]
    queryClient.setQueryData<InfiniteData<EmailPage>>(key, {
      pages: [page([email(0), email(1)]), page([email(2)])],
      pageParams: [null, 'next'],
    })
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    renderHook(() => useEmailPreviews(client), { wrapper })
    const items = () => queryClient.getQueryData<InfiniteData<EmailPage>>(key)!.pages.flatMap((p) => p.items)
    return { emit: (previews: EmailPreviews) => act(() => emit(previews)), items, queryClient }
  }

  it('patches rows on every cached page without refetching', () => {
    const { emit, items, queryClient } = setup()
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries')
    emit({ items: [
      { emailId: 'm0', preview: '第一封', previewState: 'ready' },
      { emailId: 'm2', preview: '', previewState: 'failed' },
    ] })
    expect(items().map((item) => [item.previewState, item.preview])).toEqual([['ready', '第一封'], ['missing', ''], ['failed', '']])
    expect(invalidate).not.toHaveBeenCalled()
  })

  it('treats a missing item as giving up, not as a result', () => {
    const { emit, items } = setup()
    emit({ items: [{ emailId: 'm1', preview: '', previewState: 'missing' }] })
    expect(items()[1].previewState).toBe('missing')
    render(list([email(1)], 'one', vi.fn()))
    expect(previewSlot(rows()[0])).toBeEmptyDOMElement()
  })
})
