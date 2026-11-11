import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { EmailSummary } from '@/generated/accounts'
import { SELECTION_LIMIT, useListSelection } from './use-list-selection'

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    senderGroup: null,
    id, accountId: 'work', folderId: 'inbox', subject: id,
    from: { name: 'Lin', address: 'lin@example.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

const list = ['a', 'b', 'c', 'd', 'e'].map((id) => email(id))

function setup(emails = list, scope = 'inbox') {
  return renderHook(({ emails, scope }) => useListSelection(emails, scope), { initialProps: { emails, scope } })
}

const ids = (result: { current: { items: { id: string }[] } }) => result.current.items.map((item) => item.id).sort()

describe('list selection', () => {
  it('starts with only the clicked row, never the opened one', () => {
    const { result } = setup()
    expect(result.current.active).toBe(false)
    act(() => result.current.start(list[2]))
    expect(result.current.active).toBe(true)
    expect(ids(result)).toEqual(['c'])
  })

  it('toggles rows on and off', () => {
    const { result } = setup()
    act(() => result.current.enter())
    expect(result.current.count).toBe(0)
    act(() => result.current.toggle(list[0]))
    act(() => result.current.toggle(list[3]))
    expect(ids(result)).toEqual(['a', 'd'])
    act(() => result.current.toggle(list[0]))
    expect(ids(result)).toEqual(['d'])
    expect(result.current.isSelected('d')).toBe(true)
  })

  it('picks the range from the last clicked row, in list order, both directions', () => {
    const { result } = setup()
    act(() => result.current.start(list[3]))
    act(() => result.current.extend(list[1], list))
    expect(ids(result)).toEqual(['b', 'c', 'd'])
    act(() => result.current.toggle(list[4]))
    act(() => result.current.extend(list[4], list))
    expect(ids(result)).toEqual(['b', 'c', 'd', 'e'])
  })

  it('silently drops rows that leave the list, but keeps ones that were never loaded', async () => {
    const { result, rerender } = setup()
    act(() => result.current.enter())
    await act(() => result.current.selectAll(async () => ({
      items: [...list, email('unloaded')].map(({ id, accountId, folderId, isRead, isStarred, archived, localOnly }) => ({ id, accountId, folderId, isRead, isStarred, archived, localOnly })),
      truncated: false,
    })))
    expect(result.current.count).toBe(6)
    expect(result.current.allSelected).toBe(true)
    rerender({ emails: list.filter((row) => row.id !== 'b'), scope: 'inbox' })
    expect(ids(result)).toEqual(['a', 'c', 'd', 'e', 'unloaded'])
    expect(result.current.active).toBe(true)
  })

  it('prefers the live row over the stored copy', () => {
    const { result, rerender } = setup()
    act(() => result.current.start(list[0]))
    rerender({ emails: [{ ...list[0], isRead: false }, ...list.slice(1)], scope: 'inbox' })
    expect(result.current.items[0].isRead).toBe(false)
  })

  it('clears and leaves the mode when the scope changes', () => {
    const { result, rerender } = setup()
    act(() => result.current.start(list[0]))
    rerender({ emails: list, scope: 'inbox:unread' })
    expect(result.current.active).toBe(false)
    expect(result.current.count).toBe(0)
  })

  it('ignores a select-all answer that arrives after the scope changed', async () => {
    const { result, rerender } = setup()
    act(() => result.current.enter())
    let answer!: (value: { items: never[]; truncated: boolean }) => void
    let pending!: Promise<void>
    act(() => { pending = result.current.selectAll(() => new Promise((resolve) => { answer = resolve })) })
    rerender({ emails: list, scope: 'sent' })
    await act(async () => {
      answer({ items: [], truncated: true })
      await pending
    })
    expect(result.current.active).toBe(false)
    expect(result.current.truncated).toBe(false)
  })

  it('never holds more than the limit and says so', () => {
    const many = Array.from({ length: SELECTION_LIMIT + 3 }, (_, index) => email(`m${index}`))
    const { result } = setup(many)
    act(() => result.current.start(many[0]))
    act(() => result.current.extend(many.at(-1)!, many))
    expect(result.current.count).toBe(SELECTION_LIMIT)
    expect(result.current.truncated).toBe(true)
  })
})
