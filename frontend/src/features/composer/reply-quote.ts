import type { DraftKind } from '@/generated/accounts'
import { i18n } from '@/i18n'

/** What the quote rule needs to know about one top-level block of the body. */
export type QuoteBlock = { quote: boolean; paragraph: boolean; empty: boolean }

/**
 * The quoted original of a reply (sending.md section 17.5): the last top-level
 * blockquote, plus the non-empty paragraph right before it (the core's
 * "On …, X wrote:" line). Indices are inclusive; `null` when there is no quote.
 * Shared by the editor's folding and the forgotten-attachment check, so both
 * agree on what the user wrote themselves.
 */
export function replyQuoteSpan(blocks: readonly QuoteBlock[]): { start: number; end: number } | null {
  let end = -1
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    if (blocks[index].quote) { end = index; break }
  }
  if (end < 0) return null
  const before = blocks[end - 1]
  return { start: before && before.paragraph && !before.empty ? end - 1 : end, end }
}

/** Only replies fold the original; a forward's original is the message itself. */
export function foldsReplyQuote(kind: DraftKind) {
  return kind === 'reply' || kind === 'replyAll'
}

/**
 * The divider line that opens the core's forward header ("---------- 转发的邮件
 * ----------"), in every interface language: a draft keeps the language it was
 * created in, whatever the interface uses now. Read from the same templates
 * the core renders (locales/<lang>/composeTemplates.json).
 */
function forwardDividers(): string[] {
  const languages = (i18n.options.supportedLngs || []).filter((language) => language !== 'cimode')
  return languages.flatMap((language) => {
    const template = i18n.getResource(language, 'composeTemplates', 'forwardHeader')
    const line = typeof template === 'string' ? template.split('\n').map((part) => part.trim()).find(Boolean) : undefined
    return line ? [line] : []
  })
}

/**
 * What the user wrote themselves: a reply without its quoted original (the
 * span above), a forward without the core's forward header and everything
 * after it, new mail as a whole.
 */
export function ownBodyText(html: string, kind: DraftKind): string {
  const template = document.createElement('template')
  template.innerHTML = html
  // Whitespace between blocks is not a block of its own.
  const blocks = Array.from(template.content.childNodes).filter((node) => node.nodeType !== Node.TEXT_NODE || node.textContent?.trim())
  let keep: (index: number) => boolean = () => true
  if (foldsReplyQuote(kind)) {
    const span = replyQuoteSpan(blocks.map((node) => ({
      quote: node.nodeName === 'BLOCKQUOTE',
      paragraph: node.nodeName === 'P',
      empty: !node.textContent?.trim(),
    })))
    if (span) keep = (index) => index < span.start || index > span.end
  } else if (kind === 'forward') {
    const dividers = forwardDividers()
    const header = blocks.findIndex((node) => node.nodeName === 'P' && dividers.includes(node.textContent?.trim() ?? ''))
    if (header >= 0) keep = (index) => index < header
  }
  return blocks.filter((_, index) => keep(index)).map((node) => node.textContent ?? '').join('\n')
}

/**
 * The forgotten-attachment check (sending.md section 17.6). Replies and
 * forwards inherit their subject ("Re: 合同附件"), so only new mail's subject
 * counts; the body counts only where the user wrote it.
 */
export function draftMentionsAttachment(draft: { subject: string; bodyHtml: string }, kind: DraftKind): boolean {
  return (kind === 'new' && mentionsAttachment(draft.subject)) || mentionsAttachment(ownBodyText(draft.bodyHtml, kind))
}

const CHINESE = ['附件', '附上', '附带', '随信附', '见附', '所附']
const ENGLISH = /\b(?:attach|attached|attachments?|enclosed)\b/i

/** Words that suggest the sender meant to attach a file, in either interface language. */
export function mentionsAttachment(text: string): boolean {
  return CHINESE.some((word) => text.includes(word)) || ENGLISH.test(text)
}
