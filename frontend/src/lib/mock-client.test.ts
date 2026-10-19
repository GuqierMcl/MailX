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
