import { describe, expect, it } from 'vitest'
import { emailQuery } from '@/features/mail/format'
import { createMockClient } from './mock-client'

describe('mock email pagination', () => {
  it('paginates the filtered inbox without duplicating or omitting summaries', async () => {
    const client = createMockClient()
    const query = emailQuery({ folderRole: 'inbox', unread: true, limit: 2 })
    const all = await client.listEmails({ ...query, limit: 200 })
    const ids: string[] = []
    let cursor: string | null = null
    do {
      const page = await client.listEmails({ ...query, cursor })
      expect(page.items.length).toBeLessThanOrEqual(2)
      expect(page.items.every((item) => !item.isRead)).toBe(true)
      ids.push(...page.items.map((item) => item.id))
      cursor = page.nextCursor
    } while (cursor)
    expect(ids.length).toBeGreaterThan(2)
    expect(ids).toEqual(all.items.map((item) => item.id))
    expect(new Set(ids).size).toBe(ids.length)
  })
})

describe('mock flag changes', () => {
  it('moves the folder unread number like the core does, and never below zero', async () => {
    const client = createMockClient()
    const [account] = await client.listAccounts()
    const unreadOf = async (folderId: string) =>
      (await client.listFolders(account.id)).find((folder) => folder.id === folderId)!.unreadCount
    const page = await client.listEmails({ ...emailQuery({}), accountIds: [account.id], unread: true, limit: 50 })
    const target = page.items[0]
    const before = await unreadOf(target.folderId)
    expect(await client.setFlag(target.id, 'read', true)).toBe('queued')
    expect(await unreadOf(target.folderId)).toBe(before - 1)
    // The same value again changes nothing, including the number.
    expect(await client.setFlag(target.id, 'read', true)).toBe('unchanged')
    expect(await unreadOf(target.folderId)).toBe(before - 1)
    await client.setFlag(target.id, 'read', false)
    expect(await unreadOf(target.folderId)).toBe(before)
    // A star never moves it.
    await client.setFlag(target.id, 'starred', true)
    expect(await unreadOf(target.folderId)).toBe(before)
  })
})
