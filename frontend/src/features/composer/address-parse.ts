import type { MailAddress } from '@/generated/accounts'

/** One recipient chip. An invalid one keeps what was typed in `address` and blocks sending. */
export type Recipient = { name: string | null; address: string; valid: boolean }

const MAX_NAME_CHARS = 256
const LOCAL = /^[A-Za-z0-9!#$%&'*+/=?^_`{|}~-]+(\.[A-Za-z0-9!#$%&'*+/=?^_`{|}~-]+)*$/
const LABEL = /^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?$/

/**
 * The same conservative rule as the core's `valid_address` (core/src/drafts/validate.rs):
 * one `@`, an ASCII dot-atom local part and an ASCII host name. The core stays the
 * authority; this only lets the window flag a bad address before it is saved.
 */
export function isValidAddress(address: string): boolean {
  if (address.length > 254) return false
  const at = address.indexOf('@')
  if (at < 0) return false
  const local = address.slice(0, at)
  const domain = address.slice(at + 1)
  if (!local || local.length > 64 || !LOCAL.test(local)) return false
  return domain.length > 0 && domain.length <= 253 && domain.split('.').every((label) => label.length <= 63 && LABEL.test(label))
}

const SEPARATORS = new Set([',', ';', '，', '；', '\n', '\r'])

/**
 * Splits typed or pasted text at commas, semicolons and line breaks that are not
 * inside quotes or angle brackets. `trailing` is what follows the last separator,
 * i.e. what the user is still typing.
 */
export function splitRecipients(text: string): { segments: string[]; trailing: string } {
  const segments: string[] = []
  let current = ''
  let quoted = false
  let angled = false
  for (const char of text) {
    if (char === '"' && !angled) quoted = !quoted
    else if (char === '<' && !quoted) angled = true
    else if (char === '>' && !quoted) angled = false
    if (!quoted && !angled && SEPARATORS.has(char)) {
      if (current.trim()) segments.push(current.trim())
      current = ''
    } else current += char
  }
  return { segments, trailing: current }
}

/** `Name <a@b>`, `"Name" <a@b>`, `<a@b>` or a bare address; `null` for blank text. */
export function parseRecipient(segment: string): Recipient | null {
  const text = segment.trim()
  if (!text) return null
  const angled = /^(.*?)<([^<>]*)>\s*$/.exec(text)
  let name: string | null = null
  let address: string
  if (angled) {
    name = angled[1].trim().replace(/^"(.*)"$/, '$1').replace(/\\(.)/g, '$1').trim() || null
    address = angled[2].trim()
  } else address = text.replace(/^<|>$/g, '').trim()
  const nameOk = name === null || (Array.from(name).length <= MAX_NAME_CHARS && !/\p{Cc}/u.test(name))
  return { name, address, valid: isValidAddress(address) && nameOk }
}

/** Every address in the text, the unfinished last one included. */
export function parseRecipients(text: string): Recipient[] {
  const { segments, trailing } = splitRecipients(text)
  return [...segments, trailing].flatMap((segment) => parseRecipient(segment) ?? [])
}

/** Appends `incoming`, skipping addresses already present (case-insensitive). */
export function mergeRecipients(existing: Recipient[], incoming: Recipient[]): Recipient[] {
  const seen = new Set(existing.map((item) => item.address.toLowerCase()))
  const added = incoming.filter((item) => {
    const key = item.address.toLowerCase()
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
  return added.length ? [...existing, ...added] : existing
}

export function recipientsFromAddresses(list: MailAddress[]): Recipient[] {
  return list.map((entry) => ({ name: entry.name, address: entry.address, valid: true }))
}

/** Only valid chips can be saved; the core rejects a whole draft over one bad address. */
export function toMailAddresses(list: Recipient[]): MailAddress[] {
  return list.filter((item) => item.valid).map(({ name, address }) => ({ name, address }))
}

export function formatRecipient(item: { name: string | null; address: string }) {
  return item.name ? `${item.name} <${item.address}>` : item.address
}
