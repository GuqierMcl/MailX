import type { EmailDetail, SnippetSegment } from '@/generated/accounts'

/** Same limits as the core's local search. */
export const MAX_SEARCH_TERMS = 16
export const MAX_SEARCH_CHARS = 256

/**
 * Browser-preview twin of the core's query parsing: whitespace-separated terms that
 * must all match, double quotes group a phrase. `null` means the text breaks a limit.
 */
export function parseSearchTerms(text: string): string[] | null {
  if ([...text].length > MAX_SEARCH_CHARS) return null
  const terms = [...text.matchAll(/"([^"]*)"|(\S+)/g)]
    .map((match) => (match[1] ?? match[2]).replace(/"/g, '').trim().toLowerCase())
    .filter(Boolean)
  return terms.length > MAX_SEARCH_TERMS ? null : terms
}

const SNIPPET_RADIUS = 24

function snippetAround(text: string, terms: string[]): SnippetSegment[] | null {
  const flat = text.replace(/\s+/g, ' ')
  const lower = flat.toLowerCase()
  const first = terms.map((term) => lower.indexOf(term)).filter((index) => index >= 0).sort((a, b) => a - b)[0]
  if (first === undefined) return null
  const start = Math.max(0, first - SNIPPET_RADIUS)
  const end = Math.min(flat.length, first + SNIPPET_RADIUS * 2)
  const window = flat.slice(start, end)
  const windowLower = lower.slice(start, end)
  const hits = new Array<boolean>(window.length).fill(false)
  for (const term of terms) {
    for (let at = windowLower.indexOf(term); at >= 0; at = windowLower.indexOf(term, at + term.length)) hits.fill(true, at, at + term.length)
  }
  const segments: SnippetSegment[] = []
  for (let index = 0; index < window.length; index++) {
    const last = segments.at(-1)
    if (last && last.hit === hits[index]) last.text += window[index]
    else segments.push({ text: window[index], hit: hits[index] })
  }
  if (start > 0) segments.unshift({ text: '…', hit: false })
  if (end < flat.length) segments.push({ text: '…', hit: false })
  return segments
}

/**
 * Whether the message matches every term, and the snippet when a term is only found in
 * the body or the attachment names (a hit visible in the row's own fields needs none).
 */
export function matchEmail(email: EmailDetail, terms: string[]): { matched: boolean; snippet: SnippetSegment[] | null } {
  const { summary } = email
  const header = [
    summary.subject, summary.preview,
    summary.from.name ?? '', summary.from.address,
    ...[...email.to, ...email.cc].flatMap((person) => [person.name ?? '', person.address]),
  ].join('\n').toLowerCase()
  const body = (email.body?.text ?? '').replace(/\s+/g, ' ')
  const attachments = email.attachments.map((attachment) => attachment.filename).join('\n')
  const deep = `${body}\n${attachments}`.toLowerCase()
  const matched = terms.every((term) => header.includes(term) || deep.includes(term))
  if (!matched) return { matched, snippet: null }
  const missing = terms.filter((term) => !header.includes(term))
  if (missing.length === 0) return { matched, snippet: null }
  return { matched, snippet: snippetAround(body, missing) ?? snippetAround(attachments, missing) }
}
