import { describe, expect, it } from 'vitest'
import { nowSuggestions } from './now-suggestions'
import type { NowData } from './modules/types'

const emptyData: NowData = {
  attention: [],
  draftCount: 0,
  codes: [],
  inbox: { newMail: 0, unread: 0, starredUnread: 0, rows: [] },
  attachments: [],
  activity: undefined,
  away: { sinceTime: 0, ms: 0, total: 0, senders: [] },
  unreadSenders: [],
}

describe('nowSuggestions', () => {
  it('picks up to three prompts tailored to current mailbox signals', () => {
    const picked = nowSuggestions({
      ...emptyData,
      inbox: { newMail: 4, unread: 9, starredUnread: 2, rows: [] },
      unreadSenders: [{ address: 'a@example.com', name: 'A', count: 5 }],
    })
    expect(picked.map((item) => item.key)).toEqual(['newMail', 'needReply', 'starred'])
  })

  it('falls back to general daily prompts when nothing urgent is present', () => {
    const picked = nowSuggestions(emptyData)
    expect(picked.map((item) => item.key)).toEqual(['today', 'deadlines'])
    expect(picked.every((item) => item.text.length > 0)).toBe(true)
  })
})
