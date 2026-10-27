
/**
 * A deliberately small Markdown subset for release notes, rendered as React
 * elements only: no HTML passes through, with or without markup. The notes
 * come from `latest.json`, which is not signed (the signature covers the
 * installer), so they are untrusted text.
 *
 * Supported: `#`–`###` headings, paragraphs, `-` / `*` lists, `>` quotes,
 * `**bold**`, `` `code` `` and `[text](https://…)`. Anything else — raw HTML,
 * `####`, other link schemes — is shown as it was written.
 */

/**
 * Parsing for the release notes. Kept apart from the component so the pure
 * rules can be tested and reused without React.
 */

/** Longer notes are cut here; the full text stays on GitHub. */
export const NOTES_LIMIT = 20_000

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong'; children: Inline[] }
  | { kind: 'link'; text: string; url: string }

export type Block =
  | { kind: 'heading'; level: 1 | 2 | 3; children: Inline[] }
  | { kind: 'paragraph'; children: Inline[] }
  | { kind: 'list'; items: Inline[][] }
  | { kind: 'quote'; lines: Inline[][] }

/** Only `https:` links are links; `javascript:`, `data:`, `file:` and the rest stay text. */
export function safeLink(url: string): string | null {
  const trimmed = url.trim()
  if (!/^https:\/\/[^\s/]+/i.test(trimmed)) return null
  if (trimmed.includes('\\') || [...trimmed].some((char) => char.charCodeAt(0) < 0x20)) return null
  try {
    const parsed = new URL(trimmed)
    return parsed.protocol === 'https:' && parsed.hostname.length > 0 ? trimmed : null
  } catch {
    return null
  }
}

/** Splits one line of text into text, code, bold and link runs. */
export function parseInline(text: string): Inline[] {
  const out: Inline[] = []
  let plain = ''
  const flush = () => {
    if (plain) out.push({ kind: 'text', text: plain })
    plain = ''
  }
  let rest = text
  while (rest) {
    const bold = rest.indexOf('**')
    const code = rest.indexOf('`')
    const link = rest.indexOf('[')
    const next = [bold, code, link].filter((index) => index >= 0).sort((a, b) => a - b)[0]
    if (next === undefined) {
      plain += rest
      break
    }
    plain += rest.slice(0, next)
    rest = rest.slice(next)
    if (rest.startsWith('**')) {
      const close = rest.indexOf('**', 2)
      // An unpaired `**` is text, not an empty bold run.
      if (close > 2) {
        flush()
        out.push({ kind: 'strong', children: parseInline(rest.slice(2, close)) })
        rest = rest.slice(close + 2)
      } else {
        plain += '**'
        rest = rest.slice(2)
      }
      continue
    }
    if (rest.startsWith('`')) {
      const close = rest.indexOf('`', 1)
      if (close > 1) {
        flush()
        out.push({ kind: 'code', text: rest.slice(1, close) })
        rest = rest.slice(close + 1)
      } else {
        plain += '`'
        rest = rest.slice(1)
      }
      continue
    }
    const close = rest.indexOf('](')
    const end = close > 1 ? rest.indexOf(')', close + 2) : -1
    if (close > 1 && end > close) {
      const label = rest.slice(1, close)
      const url = safeLink(rest.slice(close + 2, end))
      flush()
      // A rejected scheme keeps the words, without a way to follow them.
      out.push(url ? { kind: 'link', text: label, url } : { kind: 'text', text: label })
      rest = rest.slice(end + 1)
      continue
    }
    plain += '['
    rest = rest.slice(1)
  }
  flush()
  return out
}

const HEADING = /^(#{1,3})\s+(.*)$/
const LIST_ITEM = /^[-*]\s+(.*)$/
const QUOTE = /^>\s?(.*)$/

/** Cuts over-long notes at the cap; `truncated` tells the view to point at GitHub. */
export function truncateNotes(notes: string): { text: string; truncated: boolean } {
  return notes.length > NOTES_LIMIT
    ? { text: notes.slice(0, NOTES_LIMIT), truncated: true }
    : { text: notes, truncated: false }
}

export function parseNotes(notes: string): Block[] {
  const blocks: Block[] = []
  let paragraph: string[] = []
  const flushParagraph = () => {
    if (paragraph.length) blocks.push({ kind: 'paragraph', children: parseInline(paragraph.join('\n')) })
    paragraph = []
  }
  const lines = notes.replace(/\r\n?/g, '\n').split('\n')
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index]
    if (!line.trim()) {
      flushParagraph()
      continue
    }
    const heading = HEADING.exec(line)
    if (heading) {
      flushParagraph()
      blocks.push({ kind: 'heading', level: heading[1].length as 1 | 2 | 3, children: parseInline(heading[2]) })
      continue
    }
    if (LIST_ITEM.test(line)) {
      flushParagraph()
      const items: Inline[][] = []
      while (index < lines.length) {
        const item = LIST_ITEM.exec(lines[index])
        if (!item) break
        items.push(parseInline(item[1]))
        index += 1
      }
      index -= 1
      blocks.push({ kind: 'list', items })
      continue
    }
    if (QUOTE.test(line)) {
      flushParagraph()
      const quoted: Inline[][] = []
      while (index < lines.length) {
        const quote = QUOTE.exec(lines[index])
        if (!quote) break
        if (quote[1]) quoted.push(parseInline(quote[1]))
        index += 1
      }
      index -= 1
      blocks.push({ kind: 'quote', lines: quoted })
      continue
    }
    paragraph.push(line)
  }
  flushParagraph()
  return blocks
}

