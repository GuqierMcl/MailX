import { act, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { SNOOZED_KEY, parseSnoozed, readSnoozed, snoozeModule, unsnoozeModule, useSnoozed } from './snooze'

const noon = new Date(2026, 10, 26, 12)
const nextDay = new Date(2026, 10, 27, 8)

afterEach(() => localStorage.clear())

describe('snooze store', () => {
  it('remembers the day and the content signature', () => {
    snoozeModule('codes', 'a|b', noon)
    expect(readSnoozed(noon)).toEqual({ codes: { day: '2026-11-26', signature: 'a|b' } })
    expect(JSON.parse(localStorage.getItem(SNOOZED_KEY)!)).toEqual({ codes: { day: '2026-11-26', signature: 'a|b' } })
  })

  it('drops entries of earlier days when read, and when written', () => {
    localStorage.setItem(SNOOZED_KEY, JSON.stringify({
      old: { day: '2026-11-25', signature: 'x' },
      today: { day: '2026-11-26', signature: 'y' },
    }))
    expect(readSnoozed(noon)).toEqual({ today: { day: '2026-11-26', signature: 'y' } })
    expect(readSnoozed(nextDay)).toEqual({})
    snoozeModule('inbox', 'z', noon)
    expect(Object.keys(JSON.parse(localStorage.getItem(SNOOZED_KEY)!))).toEqual(['today', 'inbox'])
    snoozeModule('drafts', '1', nextDay)
    expect(Object.keys(JSON.parse(localStorage.getItem(SNOOZED_KEY)!))).toEqual(['drafts'])
  })

  it('tolerates anything that is not a map of well-formed entries', () => {
    for (const text of [null, '', 'garbage', '{', 'null', '7', '"x"', '[]', '{"a":1}', '{"a":null}', '{"a":{"day":5,"signature":"s"}}',
      '{"a":{"day":"2026-11-26"}}', '{"a":{"day":"today","signature":"s"}}']) {
      expect(parseSnoozed(text, noon), String(text)).toEqual({})
    }
    expect(parseSnoozed('{"a":1,"b":{"day":"2026-11-26","signature":""}}', noon)).toEqual({ b: { day: '2026-11-26', signature: '' } })
  })

  it('works over bad stored text, replacing it on the next write', () => {
    localStorage.setItem(SNOOZED_KEY, 'not json')
    expect(snoozeModule('codes', 's', noon)).toEqual({ codes: { day: '2026-11-26', signature: 's' } })
    expect(readSnoozed(noon).codes.signature).toBe('s')
  })

  it('restores a module and clears the key once nothing is left', () => {
    snoozeModule('codes', 'a', noon)
    snoozeModule('inbox', 'b', noon)
    expect(unsnoozeModule('codes', noon)).toEqual({ inbox: { day: '2026-11-26', signature: 'b' } })
    unsnoozeModule('inbox', noon)
    expect(localStorage.getItem(SNOOZED_KEY)).toBeNull()
  })

  it('keeps a changed signature for the same module', () => {
    snoozeModule('codes', 'a', noon)
    snoozeModule('codes', 'b', noon)
    expect(readSnoozed(noon)).toEqual({ codes: { day: '2026-11-26', signature: 'b' } })
  })
})

describe('useSnoozed', () => {
  it('starts from the store, follows its own writes and other windows', () => {
    const today = new Date()
    snoozeModule('codes', 'a', today)
    const { result } = renderHook(() => useSnoozed())
    expect(Object.keys(result.current.snoozed)).toEqual(['codes'])
    act(() => result.current.snooze('inbox', 'n'))
    expect(Object.keys(result.current.snoozed).sort()).toEqual(['codes', 'inbox'])
    act(() => result.current.restore('codes'))
    expect(Object.keys(result.current.snoozed)).toEqual(['inbox'])
    localStorage.removeItem(SNOOZED_KEY)
    act(() => { window.dispatchEvent(new StorageEvent('storage', { key: SNOOZED_KEY })) })
    expect(result.current.snoozed).toEqual({})
  })
})
