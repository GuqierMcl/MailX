import { describe, expect, it } from 'vitest'
import { isValidAddress, mergeRecipients, parseRecipient, parseRecipients, splitRecipients, toMailAddresses } from './address-parse'

describe('address validity (same rule as the core)', () => {
  it('accepts plain ASCII addresses and rejects what the core rejects', () => {
    for (const ok of ['a@b.co', 'first.last+tag@sub.example.org', "o'neil@example.com"]) expect(isValidAddress(ok)).toBe(true)
    for (const bad of ['', 'a', 'a@', '@b.com', 'a@@b.com', 'a b@c.com', '.a@b.com', 'a..b@c.com', 'a@-b.com', 'a@b..com', '"q"@b.com', '用户@例子.中国', 'a@b_c.com']) {
      expect(isValidAddress(bad)).toBe(false)
    }
  })
})

describe('recipient parsing', () => {
  it('splits at commas, semicolons and line breaks outside quotes and brackets', () => {
    expect(splitRecipients('a@x.com, b@y.com; "Doe, Jo" <jo@z.com>\nc@w.com').segments)
      .toEqual(['a@x.com', 'b@y.com', '"Doe, Jo" <jo@z.com>'])
    expect(splitRecipients('a@x.com, b@').trailing).toBe(' b@')
  })

  it('reads names, quoted names, angle-only and bare addresses', () => {
    expect(parseRecipient('Name <a@b.co>')).toEqual({ name: 'Name', address: 'a@b.co', valid: true })
    expect(parseRecipient('"Doe, Jo" <jo@z.com>')).toEqual({ name: 'Doe, Jo', address: 'jo@z.com', valid: true })
    expect(parseRecipient('<a@b.co>')).toEqual({ name: null, address: 'a@b.co', valid: true })
    expect(parseRecipient('  c@d.org ')).toEqual({ name: null, address: 'c@d.org', valid: true })
    expect(parseRecipient('   ')).toBeNull()
  })

  it('keeps invalid input as an invalid recipient instead of dropping it', () => {
    expect(parseRecipient('not-an-address')).toEqual({ name: null, address: 'not-an-address', valid: false })
    expect(parseRecipient('Name <bad@@x>')?.valid).toBe(false)
  })

  it('parses a pasted list, including the unfinished tail', () => {
    expect(parseRecipients('Name <a@b.co>, c@d.org').map((item) => item.address)).toEqual(['a@b.co', 'c@d.org'])
  })

  it('merges without duplicates (case-insensitive) and saves only valid chips', () => {
    const existing = [{ name: null, address: 'A@b.co', valid: true }]
    const merged = mergeRecipients(existing, [{ name: 'X', address: 'a@B.co', valid: true }, { name: null, address: 'bad', valid: false }])
    expect(merged.map((item) => item.address)).toEqual(['A@b.co', 'bad'])
    expect(toMailAddresses(merged)).toEqual([{ name: null, address: 'A@b.co' }])
  })
})
