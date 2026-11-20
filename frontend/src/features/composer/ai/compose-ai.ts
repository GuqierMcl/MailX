import { generateHTML, type Editor } from '@tiptap/core'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import type { DraftKind } from '@/generated/accounts'
import { filterClipboardHtml } from '../mail-body-editor-schema'
import { findReplyQuote } from '../reply-quote-fold'
import { foldsReplyQuote, forwardHeaderIndex } from '../reply-quote'

/** Tags that mark the context for the model; a model sometimes echoes them back. */
const WRAPPER = /^\s*<(draft|text|original_email|instruction)>\s*/i
const WRAPPER_END = /\s*<\/(draft|text|original_email|instruction)>\s*$/i

/** The answer without the section tags a model copied from its input. */
export function withoutWrapperTags(text: string): string {
  return text.replace(WRAPPER, '').replace(WRAPPER_END, '').trim()
}

/**
 * One line break where the answer meant a new paragraph. Models often separate paragraphs with a
 * single newline; Markdown reads that as a soft break inside one paragraph, so a taken draft lost the
 * spacing the preview showed. Lists and quotes keep their own lines together.
 */
export function normalizeParagraphs(markdown: string): string {
  const lines = markdown.replace(/\r\n?/g, '\n').split('\n')
  const out: string[] = []
  let inList: boolean = false
  for (const line of lines) {
    if (!line.trim()) { out.push(''); inList = false; continue }
    const listItem = /^\s*(?:[-*+]|\d+[.)])\s/.test(line)
    const indented = /^\s{2,}/.test(line)
    const continuation: boolean = inList && indented
    const previous = out[out.length - 1]
    if (previous !== undefined && previous !== '' && !(listItem && inList) && !continuation) out.push('')
    out.push(line)
    inList = listItem || /^\s*>/.test(line) || continuation
  }
  // Two or more blank lines are one blank line, as in the model's output.
  return out.join('\n').replace(/\n{3,}/g, '\n\n').trim()
}

/** A range of the document: `[from, to)`. */
export type DocRange = { from: number; to: number }

/**
 * Where the user's own writing ends (ai-chat-ui.md 5.3): before a reply's quoted
 * original, before a forward's header, or the end of new mail. Writing help never
 * sends or changes anything after it.
 */
export function ownEnd(doc: ProseMirrorNode, kind: DraftKind): number {
  if (foldsReplyQuote(kind)) {
    const quote = findReplyQuote(doc)
    if (quote) return quote.from
  } else if (kind === 'forward') {
    const texts: string[] = []
    const positions: number[] = []
    doc.forEach((node, offset) => { texts.push(node.type.name === 'paragraph' ? node.textContent.trim() : ''); positions.push(offset) })
    const header = forwardHeaderIndex(texts)
    if (header >= 0) return positions[header]
  }
  return doc.content.size
}

/** The user's own part, or `null` when it holds no text. */
export function ownRange(doc: ProseMirrorNode, kind: DraftKind): DocRange | null {
  const to = ownEnd(doc, kind)
  return doc.textBetween(0, to, '\n').trim() ? { from: 0, to } : null
}

/** Inline images do not survive Markdown: a range with one is not rewritten. */
export function hasImage(doc: ProseMirrorNode, range: DocRange): boolean {
  let found = false
  doc.nodesBetween(range.from, range.to, (node) => {
    if (node.type.name === 'inlineImage') found = true
    return !found
  })
  return found
}

/** The range as Markdown, the form the model reads and writes. */
export function rangeMarkdown(editor: Editor, range: DocRange): string {
  const slice = editor.state.doc.slice(range.from, range.to)
  const content = slice.content.toJSON() as object[] | null
  // Inline content (part of one paragraph) needs a paragraph around it to serialize.
  const blocks = slice.openStart > 0 && slice.content.firstChild?.isInline ? [{ type: 'paragraph', content }] : content ?? []
  return editor.markdown!.serialize({ type: 'doc', content: blocks }).trim()
}

/** Model Markdown as the editor's HTML, through the same filter as a paste. */
export function markdownHtml(editor: Editor, markdown: string): string {
  const json = editor.markdown!.parse(markdown)
  return filterClipboardHtml(generateHTML(json, editor.extensionManager.baseExtensions))
}

/**
 * The one form of an answer (ai-chat-ui.md 5.3): the section tags a model echoed and the output
 * dialect of paragraphs are settled here, then the editor's own parser and filter turn it into
 * HTML. The preview shows this string and taking it inserts this same string, so what the card
 * shows is, by construction, what the draft gets.
 */
export function answerHtml(editor: Editor, text: string): string {
  // A blank line is the reader's space between paragraphs, so it becomes an empty paragraph of
  // its own, as it would when the user pressed Enter twice. Without one, blocks sit one under the
  // other, like lines the user typed back to back.
  // Split on the blank lines the model wrote first: normalizing then turns each block's single
  // newlines into paragraphs without inventing empties the model did not write.
  return withoutWrapperTags(text)
    .replace(/\r\n?/g, '\n')
    .split(/\n[ \t]*\n/)
    .map((block) => normalizeParagraphs(block))
    .filter((block) => block !== '')
    .map((block) => markdownHtml(editor, block))
    .join('<p></p>')
}

/** The text a reader sees, one line per block; for comparing a proofread. */
export function answerText(editor: Editor, text: string): string {
  return htmlText(answerHtml(editor, text))
}

function htmlText(html: string): string {
  const template = document.createElement('template')
  template.innerHTML = html
  const lines: string[] = []
  const walk = (node: Node) => {
    for (const child of Array.from(node.childNodes)) {
      if (child instanceof HTMLElement && ['P', 'LI', 'H1', 'H2', 'H3'].includes(child.tagName)) {
        const text = child.textContent ?? ''
        // A list item's own paragraph is read once.
        if (child.tagName !== 'LI' || !child.querySelector('p')) lines.push(text)
        else walk(child)
      } else if (child instanceof HTMLElement) walk(child)
    }
  }
  walk(template.content)
  return lines.join('\n')
}

/** The end of the top-level block holding `position`: where a preview or a new block goes. */
export function blockEnd(doc: ProseMirrorNode, position: number): number {
  const resolved = doc.resolve(Math.min(position, doc.content.size))
  return resolved.depth >= 1 ? resolved.after(1) : position
}

/** The empty top-level block holding `position`, if it is one: a draft replaces it. */
export function emptyBlock(doc: ProseMirrorNode, position: number): DocRange | null {
  const resolved = doc.resolve(Math.min(position, doc.content.size))
  if (resolved.depth < 1) return null
  const block = resolved.node(1)
  return block.textContent.trim() || block.childCount > 1 ? null : { from: resolved.before(1), to: resolved.after(1) }
}

export type DiffPart = { kind: 'same' | 'removed' | 'added'; text: string }

/** Words, spaces and punctuation; CJK characters one by one. */
function tokens(text: string): string[] {
  return text.match(/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]|[\p{L}\p{N}'’]+|\s+|./gu) ?? []
}

/** Above this many token pairs the comparison is skipped and the whole new text shown. */
const MAX_CELLS = 4_000_000

/**
 * What a proofread changed, token by token (longest common subsequence after
 * trimming the shared start and end). `null` when the texts are too long to compare.
 */
export function diff(before: string, after: string): DiffPart[] | null {
  const a = tokens(before)
  const b = tokens(after)
  let start = 0
  while (start < a.length && start < b.length && a[start] === b[start]) start += 1
  let endA = a.length
  let endB = b.length
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) { endA -= 1; endB -= 1 }
  const n = endA - start
  const m = endB - start
  if (n * m > MAX_CELLS) return null
  // lengths[i][j]: common tokens of a[start+i..] and b[start+j..], flattened.
  const width = m + 1
  const lengths = new Uint32Array((n + 1) * width)
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      lengths[i * width + j] = a[start + i] === b[start + j]
        ? lengths[(i + 1) * width + j + 1] + 1
        : Math.max(lengths[(i + 1) * width + j], lengths[i * width + j + 1])
    }
  }
  const parts: DiffPart[] = []
  const push = (kind: DiffPart['kind'], text: string) => {
    const last = parts[parts.length - 1]
    if (last?.kind === kind) last.text += text
    else parts.push({ kind, text })
  }
  if (start > 0) push('same', a.slice(0, start).join(''))
  let i = 0
  let j = 0
  while (i < n || j < m) {
    if (i < n && j < m && a[start + i] === b[start + j]) { push('same', a[start + i]); i += 1; j += 1 }
    else if (j < m && (i === n || lengths[i * width + j + 1] >= lengths[(i + 1) * width + j])) { push('added', b[start + j]); j += 1 }
    else { push('removed', a[start + i]); i += 1 }
  }
  if (endA < a.length) push('same', a.slice(endA).join(''))
  return parts
}
