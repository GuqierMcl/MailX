import { describe, expect, it } from 'vitest'
import { capsuleSuggestions, SUGGESTION_COUNT } from './capsule-suggestions'

/** A fixed sequence standing in for Math.random. */
const sequence = (...values: number[]) => { let index = 0; return () => values[index++ % values.length] }

describe('capsule suggestions', () => {
  it('pick three distinct requests for the view, differently with another roll', () => {
    const first = capsuleSuggestions({ type: 'email', emailId: 'e' }, { sender: 'Alice' }, sequence(0, 0, 0))
    const second = capsuleSuggestions({ type: 'email', emailId: 'e' }, { sender: 'Alice' }, sequence(0.99, 0.99, 0.99))
    expect(first).toHaveLength(SUGGESTION_COUNT)
    expect(new Set(first.map((pick) => pick.key)).size).toBe(SUGGESTION_COUNT)
    expect(first.map((pick) => pick.key)).not.toEqual(second.map((pick) => pick.key))
  })

  it('every pick is eventually offered, and none repeats within one opening', () => {
    const seen = new Set<string>()
    for (let roll = 0; roll < 200; roll += 1) {
      const picks = capsuleSuggestions(null, { sender: 'Bob' })
      expect(new Set(picks.map((pick) => pick.key)).size).toBe(picks.length)
      for (const pick of picks) seen.add(pick.key)
    }
    expect(seen.size).toBe(8)
  })

  it('names a sender only when there is one, shortened', () => {
    for (let roll = 0; roll < 50; roll += 1) {
      expect(capsuleSuggestions(null, { sender: '  ' }).map((pick) => pick.key)).not.toContain('latestFrom')
    }
    const named = capsuleSuggestions(null, { sender: '一个非常非常非常非常非常非常非常长的发件人名字啊' }, sequence(0))
    expect(named[0].key).toBe('latestFrom')
    expect(named[0].text).toContain('…')
  })

  it('counts the selection and fits each kind of view', () => {
    const selected = capsuleSuggestions({ type: 'emails', emailIds: ['a', 'b', 'c'] }, { count: 3 }, sequence(0))
    expect(selected[0]).toEqual({ key: 'summarize', text: '总结选中的 3 封邮件' })
    const draft = capsuleSuggestions({ type: 'draft', draftId: 'd' }, {}, sequence(0))
    expect(draft[0].key).toBe('review')
  })
})
