/**
 * A small Markdown subset for the assistant's replies (ai-chat-ui.md 4.2). It produces plain data,
 * never HTML: paragraphs, line breaks, bulleted and numbered lists, bold, italic, inline code, code
 * blocks, blockquotes and links. Raw HTML stays text, images are dropped, and a link survives only
 * with an http, https or mailto address. Unfinished input (a reply still streaming) never throws:
 * an unclosed construct stays literal, or with `open` is closed at the end of the text.
 */

export type Inline =
  | { type: 'text'; text: string }
  | { type: 'br' }
  | { type: 'code'; text: string }
  | { type: 'strong'; children: Inline[] }
  | { type: 'em'; children: Inline[] }
  | { type: 'link'; href: string; children: Inline[] }

export type MarkdownBlock =
  | { type: 'paragraph'; children: Inline[] }
  | { type: 'heading'; children: Inline[] }
  | { type: 'code'; text: string }
  | { type: 'quote'; children: MarkdownBlock[] }
  | { type: 'list'; ordered: boolean; start: number; items: MarkdownBlock[][] }

/** The address of a link the app may open, or null: only http(s) and mailto, nothing else. */
export function safeHref(raw: string): string | null {
  const value = raw.trim()
  if (value === '' || /\s/.test(value) || [...value].some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127)) return null
  try {
    const url = new URL(value)
    if (url.protocol === 'http:' || url.protocol === 'https:') return url.hostname === '' ? null : url.href
    if (url.protocol === 'mailto:') return url.pathname === '' ? null : value
  } catch { /* not an absolute address */ }
  return null
}

const FENCE = /^\s{0,3}(```|~~~)/
const LIST_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/
const HEADING = /^\s{0,3}#{1,6}\s+(.*)$/
const QUOTE = /^\s{0,3}>\s?(.*)$/
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/

/**
 * Cuts the text into top-level sources at blank lines, keeping a code fence in one piece even when it
 * holds blank lines. All but the last piece are final: later text never changes them, so a renderer
 * can parse and keep them once and only redo the last piece while the reply grows.
 */
export function splitBlocks(text: string): string[] {
  const pieces: string[] = []
  let current: string[] = []
  let fence: string | null = null
  const flush = () => {
    if (current.length > 0) pieces.push(current.join('\n'))
    current = []
  }
  for (const line of text.replace(/\r\n?/g, '\n').split('\n')) {
    if (fence !== null) {
      current.push(line)
      if (new RegExp(`^\\s{0,3}${fence}\\s*$`).test(line)) { fence = null; flush() }
      continue
    }
    const opening = FENCE.exec(line)
    if (opening) {
      // A fence ends the paragraph before it.
      flush()
      fence = opening[1]
      current.push(line)
      continue
    }
    if (line.trim() === '') { flush(); continue }
    current.push(line)
  }
  flush()
  return pieces
}

/** Parses the whole text. `open` closes what a streaming reply has left open. */
export function parseMarkdown(text: string, open = false): MarkdownBlock[] {
  const pieces = splitBlocks(text)
  return pieces.flatMap((piece, index) => parseBlock(piece, open && index === pieces.length - 1))
}

/** One piece from `splitBlocks`: it may still hold a list, quote or heading next to a paragraph. */
export function parseBlock(source: string, open = false): MarkdownBlock[] {
  const lines = source.split('\n')
  const blocks: MarkdownBlock[] = []
  let at = 0
  while (at < lines.length) {
    const line = lines[at]
    const fence = FENCE.exec(line)
    if (fence) {
      const marker = fence[1]
      const body: string[] = []
      at += 1
      while (at < lines.length && !new RegExp(`^\\s{0,3}${marker}\\s*$`).test(lines[at])) { body.push(lines[at]); at += 1 }
      at += 1
      blocks.push({ type: 'code', text: body.join('\n') })
      continue
    }
    if (RULE.test(line) && !LIST_ITEM.test(line)) { at += 1; continue }
    const heading = HEADING.exec(line)
    if (heading) {
      at += 1
      blocks.push({ type: 'heading', children: parseInline(heading[1].replace(/\s+#+\s*$/, ''), open && at >= lines.length) })
      continue
    }
    if (QUOTE.test(line)) {
      const inner: string[] = []
      while (at < lines.length && QUOTE.test(lines[at])) { inner.push(QUOTE.exec(lines[at])![1]); at += 1 }
      blocks.push({ type: 'quote', children: parseMarkdown(inner.join('\n'), open && at >= lines.length) })
      continue
    }
    const item = LIST_ITEM.exec(line)
    if (item) {
      const base = item[1].length
      const ordered = /\d/.test(item[2])
      const start = ordered ? Number.parseInt(item[2], 10) : 1
      const items: MarkdownBlock[][] = []
      while (at < lines.length) {
        const entry = LIST_ITEM.exec(lines[at])
        if (!entry || entry[1].length !== base || /\d/.test(entry[2]) !== ordered) break
        const body = [entry[3]]
        at += 1
        // Lines indented past the marker belong to this item (a nested list, a wrapped line).
        while (at < lines.length && lines[at].trim() !== '' && indent(lines[at]) > base) {
          body.push(lines[at].slice(Math.min(indent(lines[at]), base + 2)))
          at += 1
        }
        items.push(parseBlock(body.join('\n'), open && at >= lines.length))
      }
      blocks.push({ type: 'list', ordered, start, items })
      continue
    }
    const paragraph: string[] = []
    while (at < lines.length && (paragraph.length === 0 || !startsBlock(lines[at]))) { paragraph.push(lines[at]); at += 1 }
    blocks.push({ type: 'paragraph', children: parseInline(paragraph.map((row) => row.trim()).join('\n'), open && at >= lines.length) })
  }
  return blocks
}

const indent = (line: string) => line.length - line.trimStart().length
const startsBlock = (line: string) => FENCE.test(line) || HEADING.test(line) || QUOTE.test(line) || LIST_ITEM.test(line)

const ESCAPABLE = '\\`*_{}[]()#+-.!<>~|'

/** Inline text. `open` ends unclosed bold, italic, code and links at the end of the text. */
export function parseInline(text: string, open = false): Inline[] {
  const out: Inline[] = []
  let literal = ''
  const flush = () => {
    if (literal !== '') out.push({ type: 'text', text: literal })
    literal = ''
  }
  const push = (node: Inline) => { flush(); out.push(node) }
  let at = 0
  while (at < text.length) {
    const char = text[at]
    const rest = text.slice(at)
    if (char === '\\' && at + 1 < text.length && ESCAPABLE.includes(text[at + 1])) { literal += text[at + 1]; at += 2; continue }
    if (char === '\n') { push({ type: 'br' }); at += 1; continue }
    if (char === '`') {
      const ticks = /^`+/.exec(rest)![0]
      const close = text.indexOf(ticks, at + ticks.length)
      if (close > at + ticks.length) { push({ type: 'code', text: text.slice(at + ticks.length, close).replace(/^ (.*) $/s, '$1') }); at = close + ticks.length; continue }
      if (open && close < 0 && text.slice(at + ticks.length).trim() !== '') { push({ type: 'code', text: text.slice(at + ticks.length) }); at = text.length; continue }
      literal += ticks; at += ticks.length; continue
    }
    if (char === '!' && text[at + 1] === '[') {
      // Images are never loaded: only the description stays.
      const image = linkAt(text, at + 1)
      if (image) { literal += image.label; at = image.end; continue }
    }
    if (char === '[') {
      const link = linkAt(text, at)
      if (link) {
        const href = safeHref(link.url)
        if (href) push({ type: 'link', href, children: parseInline(link.label, false) })
        else literal += link.label
        at = link.end
        continue
      }
    }
    if (char === '<') {
      const auto = /^<((?:https?:\/\/|mailto:)[^\s<>]+)>/.exec(rest)
      if (auto) {
        const href = safeHref(auto[1])
        if (href) push({ type: 'link', href, children: [{ type: 'text', text: auto[1] }] })
        else literal += auto[0]
        at += auto[0].length
        continue
      }
    }
    if ((char === 'h' || char === 'H') && (at === 0 || /[\s([{>"']/.test(text[at - 1]))) {
      const bare = /^https?:\/\/[^\s<>]+/i.exec(rest)
      if (bare) {
        // Closing brackets and sentence punctuation after the address are not part of it.
        let url = bare[0]
        while (/[.,;:!?'"\])}。，；：！？）】」』]$/.test(url) && !(url.endsWith(')') && url.includes('('))) url = url.slice(0, -1)
        const href = safeHref(url)
        if (href) { push({ type: 'link', href, children: [{ type: 'text', text: url }] }); at += url.length; continue }
      }
    }
    if (char === '*' || char === '_') {
      const run = /^[*_]+/.exec(rest)![0]
      const marks = run[0] === '*' || (at === 0 || !/\w/.test(text[at - 1])) ? Math.min(run.length, 2) : 0
      if (marks > 0 && run === char.repeat(run.length) && run.length <= 3) {
        const delimiter = char.repeat(marks)
        const after = text[at + marks]
        const opens = after !== undefined && !/\s/.test(after)
        if (opens) {
          const end = closing(text, at + marks, delimiter)
          if (end > at + marks) {
            push({ type: marks === 2 ? 'strong' : 'em', children: parseInline(text.slice(at + marks, end), false) })
            at = end + marks
            continue
          }
          if (open && end < 0 && text.slice(at + marks).trim() !== '') {
            push({ type: marks === 2 ? 'strong' : 'em', children: parseInline(text.slice(at + marks), true) })
            at = text.length
            continue
          }
        }
      }
      literal += run; at += run.length; continue
    }
    literal += char
    at += 1
  }
  flush()
  return out
}

/** The index of the delimiter that closes an emphasis opened before `from`, or -1. */
function closing(text: string, from: number, delimiter: string): number {
  const mark = delimiter[0]
  for (let at = from; at < text.length; at += 1) {
    if (text[at] === '\\') { at += 1; continue }
    if (text[at] === '`') {
      const ticks = /^`+/.exec(text.slice(at))![0]
      const end = text.indexOf(ticks, at + ticks.length)
      if (end > 0) at = end + ticks.length - 1
      continue
    }
    if (text[at] !== mark || text.slice(at, at + delimiter.length) !== delimiter) continue
    if (text[at - 1] === undefined || /\s/.test(text[at - 1])) continue
    // `_` inside a word is not a closer.
    if (mark === '_' && /\w/.test(text[at + delimiter.length] ?? '')) continue
    // A longer run of the same mark closes its own, not this one.
    if (delimiter.length === 1 && text[at + 1] === mark) { at += 1; continue }
    return at
  }
  return -1
}

/** `[label](url)` starting at `start`, with balanced brackets and parentheses, or null. */
function linkAt(text: string, start: number): { label: string; url: string; end: number } | null {
  let depth = 0
  let at = start
  for (; at < text.length; at += 1) {
    if (text[at] === '\\') { at += 1; continue }
    if (text[at] === '[') depth += 1
    else if (text[at] === ']') { depth -= 1; if (depth === 0) break }
  }
  if (depth !== 0 || text[at + 1] !== '(') return null
  const label = text.slice(start + 1, at)
  let paren = 1
  let end = at + 2
  for (; end < text.length; end += 1) {
    if (text[end] === '\\') { end += 1; continue }
    if (text[end] === '(') paren += 1
    else if (text[end] === ')') { paren -= 1; if (paren === 0) break }
    else if (text[end] === '\n') return null
  }
  if (paren !== 0) return null
  // An optional title after the address is dropped.
  const url = text.slice(at + 2, end).trim().replace(/\s+["'].*["']$/, '')
  return { label, url, end: end + 1 }
}
