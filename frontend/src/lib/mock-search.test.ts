import { describe, expect, it } from 'vitest'
import type { EmailDetail } from '@/generated/accounts'
import { matchEmail, parseSearchTerms } from './mock-search'
import { createMockClient } from './mock-client'

function detail(overrides: { subject?: string; preview?: string; body?: string | null; attachment?: string } = {}): EmailDetail {
  return {
    summary: {
      id: 'm', accountId: 'a', folderId: 'f', subject: overrides.subject ?? '周报', from: { name: '林巍', address: 'lin@example.com' },
      preview: overrides.preview ?? '本周进展', receivedAt: '2026-10-20T00:00:00Z', isRead: true, isStarred: false, hasAttachment: false,
      bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false,
    },
    to: [{ name: null, address: 'me@example.com' }], cc: [], replyTo: [],
    body: overrides.body === null ? null : { text: overrides.body ?? '合同已经签署，请查收附件。' },
    attachments: overrides.attachment ? [{ id: 'x', filename: overrides.attachment, mimeType: 'text/plain', size: 1, isInline: false }] : [],
  } as unknown as EmailDetail
}

describe('mock search terms', () => {
  it('splits on whitespace and groups quoted phrases', () => {
    expect(parseSearchTerms('Foo  "bar baz" 合同')).toEqual(['foo', 'bar baz', '合同'])
  })

  it('rejects more than 16 terms or 256 characters', () => {
    expect(parseSearchTerms(Array.from({ length: 17 }, (_, i) => `t${i}`).join(' '))).toBeNull()
    expect(parseSearchTerms('a'.repeat(257))).toBeNull()
    expect(parseSearchTerms('a'.repeat(256))).not.toBeNull()
  })
})

describe('mock search matching', () => {
  it('needs every term (AND), case-insensitively', () => {
    expect(matchEmail(detail(), parseSearchTerms('周报 LIN')!).matched).toBe(true)
    expect(matchEmail(detail(), ['周报', '不存在']).matched).toBe(false)
  })

  it('matches a quoted phrase only as a whole', () => {
    expect(matchEmail(detail({ body: 'the quarterly report is ready' }), ['quarterly report']).matched).toBe(true)
    expect(matchEmail(detail({ body: 'the report is quarterly' }), ['quarterly report']).matched).toBe(false)
  })

  it('finds a two-character Chinese term in the body and marks it in the snippet', () => {
    const { matched, snippet } = matchEmail(detail(), ['合同'])
    expect(matched).toBe(true)
    expect(snippet?.filter((s) => s.hit).map((s) => s.text)).toEqual(['合同'])
    expect(snippet?.map((s) => s.text).join('')).toContain('合同已经签署')
  })

  it('gives no snippet when the hit is in the row itself, and one for an attachment name', () => {
    expect(matchEmail(detail(), ['周报']).snippet).toBeNull()
    const named = matchEmail(detail({ attachment: '报价单.pdf' }), ['报价'])
    expect(named.snippet?.find((s) => s.hit)?.text).toBe('报价')
  })

  it('is served by the mock client with the indexed count', async () => {
    const client = createMockClient()
    const all = await client.listEmails({ limit: 500 } as never)
    expect(all.coverage.indexedBodies).toBeGreaterThanOrEqual(0)
    await expect(client.listEmails({ text: Array.from({ length: 17 }, (_, i) => `t${i}`).join(' ') } as never))
      .rejects.toMatchObject({ failure: { code: 'validation', field: 'text' } })
    const none = await client.listEmails({ text: 'zzzz-no-such-text' } as never)
    expect(none.items).toEqual([])
  })
})
