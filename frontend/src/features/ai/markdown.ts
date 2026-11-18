/**
 * Markdown for the assistant's replies (ai-chat-ui.md 4.2). `marked`'s lexer cuts the text into
 * tokens (GFM, a single newline is a line break); this module turns them into plain data, never HTML,
 * and `markdown-view.tsx` draws that data as React elements. Raw HTML stays text, images keep only
 * their description, and a link survives only with an http, https or mailto address; a `mailx://`
 * link is a reference to local mail or a folder. Unfinished input (a reply still streaming) never
 * throws: `closeOpenMarkdown` closes what the end of the text has left open before it is read.
 */
import { Lexer, Tokenizer, type Links, type Token, type Tokens } from 'marked'

export type Inline =
  | { type: 'text'; text: string }
  | { type: 'br' }
  | { type: 'code'; text: string }
  | { type: 'strong'; children: Inline[] }
  | { type: 'em'; children: Inline[] }
  | { type: 'del'; children: Inline[] }
  | { type: 'link'; href: string; children: Inline[] }
  | { type: 'ref'; kind: RefKind; id: string; label: string }

export type HeadingLevel = 1 | 2 | 3 | 4 | 5 | 6
export type TableAlign = 'left' | 'center' | 'right' | null
/** One list entry. `checked` is null for a plain item, true or false for a task item. */
export type ListItem = { checked: boolean | null; blocks: MarkdownBlock[] }

export type MarkdownBlock =
  | { type: 'paragraph'; children: Inline[] }
  | { type: 'heading'; level: HeadingLevel; children: Inline[] }
  | { type: 'code'; lang: string | null; text: string }
  /** A reference that stands alone on its line: shown as a card. */
  | { type: 'ref'; kind: RefKind; id: string; label: string }
  | { type: 'quote'; children: MarkdownBlock[] }
  | { type: 'list'; ordered: boolean; start: number; loose: boolean; items: ListItem[] }
  | { type: 'table'; align: TableAlign[]; header: Inline[][]; rows: Inline[][][] }
  | { type: 'rule' }

/** What a `mailx://` link in a reply can point to: a mail or a folder of this app. */
export type RefKind = 'email' | 'folder'

const REF_ID: Record<RefKind, RegExp> = {
  // Core ids are UUIDs; the mock data's ids are short words and numbers. Nothing else (no spaces, slashes or dots-only).
  email: /^[A-Za-z0-9][A-Za-z0-9:_-]{0,127}$/,
  // A folder id may carry a path (`<account>:projects/mailx` in the mock data).
  folder: /^[A-Za-z0-9][A-Za-z0-9:_/-]{0,127}$/,
}

/** `mailx://email/<id>` or `mailx://folder/<id>` as a reference, or null: any other `mailx:` address is not one. */
export function parseRef(url: string, label: string): Extract<Inline, { type: 'ref' }> | null {
  const match = /^mailx:\/\/(email|folder)\/([^\s?#]+)$/.exec(url.trim())
  if (!match) return null
  const kind = match[1] as RefKind
  return REF_ID[kind].test(match[2]) ? { type: 'ref', kind, id: match[2], label } : null
}

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

// --- Entities ---------------------------------------------------------------------------------

/** The named entities a model plausibly writes; any other name stays as written. */
const ENTITIES: Record<string, string> = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: '\'', nbsp: '\u00a0', copy: '©', reg: '®', trade: '™', hellip: '…',
  mdash: '—', ndash: '–', lsquo: '‘', rsquo: '’', ldquo: '“', rdquo: '”', laquo: '«', raquo: '»', middot: '·', bull: '•',
  times: '×', divide: '÷', minus: '−', plusmn: '±', deg: '°', para: '¶', sect: '§', euro: '€', pound: '£', yen: '¥', cent: '¢',
  larr: '←', rarr: '→', uarr: '↑', darr: '↓', harr: '↔', lArr: '⇐', rArr: '⇒', le: '≤', ge: '≥', ne: '≠', asymp: '≈',
  infin: '∞', check: '✓', hearts: '♥', star: '☆', zwj: '\u200d', zwnj: '\u200c', shy: '\u00ad', ensp: '\u2002', emsp: '\u2003', thinsp: '\u2009',
}

/** Decodes HTML entities (`&amp;`, `&#39;`, `&#x1F600;`) to the characters they name. Unknown names stay literal. */
export function decodeEntities(text: string): string {
  if (!text.includes('&')) return text
  return text.replace(/&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[A-Za-z][A-Za-z0-9]{1,31});/g, (whole, name: string) => {
    if (name[0] !== '#') return ENTITIES[name] ?? whole
    const code = name[1] === 'x' || name[1] === 'X' ? Number.parseInt(name.slice(2), 16) : Number.parseInt(name.slice(1), 10)
    // NUL, surrogates and out-of-range code points become the replacement character, as in HTML.
    if (code === 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)) return '\uFFFD'
    return String.fromCodePoint(code)
  })
}

// --- Lexing -----------------------------------------------------------------------------------

/** Sentence punctuation that ends a bare address even with no space after it (CJK text has none). */
const URL_STOP = /[。，、；：！？）】」』《》（【「『]/
const URL_TRAILING = /[.,;:!?'"\])}]$/

/**
 * GFM's bare-address rule, with one change: an address ends at CJK punctuation, so
 * `https://a.com/x。下一句` links `https://a.com/x` and keeps `。下一句` as text.
 */
class ReplyTokenizer extends Tokenizer {
  override url(src: string): Tokens.Link | undefined {
    const token = super.url(src)
    if (!token || token.href.startsWith('mailto:')) return token
    let raw = token.raw
    const stop = raw.search(URL_STOP)
    if (stop >= 0) raw = raw.slice(0, stop)
    while (URL_TRAILING.test(raw) && !(raw.endsWith(')') && raw.includes('('))) raw = raw.slice(0, -1)
    if (raw === token.raw) return token
    if (!/^(?:https?:\/\/|www\.)[^/?#]*[A-Za-z0-9]/i.test(raw)) return undefined
    const href = token.href.startsWith('http://') && !raw.startsWith('http') ? `http://${raw}` : raw
    return { type: 'link', raw, text: raw, href, tokens: [{ type: 'text', raw, text: raw }] }
  }
}

function newLexer(links?: Links): Lexer {
  const lexer = new Lexer({ gfm: true, breaks: true, tokenizer: new ReplyTokenizer() })
  if (links) Object.assign(lexer.tokens.links, links)
  return lexer
}

const normalize = (text: string) => text.replace(/\r\n?/g, '\n')

/** A top-level token of a reply as the view keeps it: its source, and whether it is the part still being written. */
export type MarkdownPiece = { raw: string; token: Token; open: boolean }

/** What the incremental reader carries from one update to the next. */
export type LexState = {
  text: string
  /** Top-level tokens that later text can no longer change. */
  done: Token[]
  /** Where the text after `done` starts. */
  doneLength: number
  /** The tokens later text may still change (see `openFrom`): lexed again on every update. */
  tail: Token[]
  /** Reference definitions in `done`: the tail is lexed with these, as a full lex would know them. */
  doneLinks: Links
  /** False when token sources did not add up to the text: every update then reads the whole text. */
  incremental: boolean
}

const hasDef = (tokens: Token[]): boolean => tokens.some((token) =>
  token.type === 'def' ||
  ('tokens' in token && Array.isArray(token.tokens) && hasDef(token.tokens)) ||
  (token.type === 'list' && (token as Tokens.List).items.some((item) => hasDef(item.tokens))))

/** The reference definitions among `tokens`, the first of a name winning as in marked. */
function collectLinks(tokens: Token[], into: Links): Links {
  for (const token of tokens) {
    if (token.type === 'def') {
      const def = token as Tokens.Def
      if (!(def.tag in into)) into[def.tag] = { href: def.href, title: def.title }
    } else if (token.type === 'list') for (const item of (token as Tokens.List).items) collectLinks(item.tokens, into)
    else if ('tokens' in token && Array.isArray(token.tokens)) collectLinks(token.tokens, into)
  }
  return into
}

/**
 * Where the part that later text may still change begins. That is the last block, together with any
 * block right before it with no blank line between (marked may still regroup lines across them) and a
 * list before it (a paragraph after a list can still become its next item, `2` turning into `2.`).
 */
function openFrom(tokens: Token[]): number {
  let cut = tokens.length - 1
  while (cut > 0 && tokens[cut].type === 'space') cut -= 1
  for (;;) {
    if (cut > 0 && tokens[cut - 1].type !== 'space') { cut -= 1; continue }
    const before = cut - 2
    if (before >= 0 && tokens[before].type === 'list') { cut = before; continue }
    return cut
  }
}

function settle(text: string, done: Token[], doneLinks: Links, offset: number, tokens: Token[]): LexState {
  let cut = openFrom(tokens)
  cut = Math.max(cut, 0)
  const length = tokens.reduce((sum, token) => sum + token.raw.length, 0)
  if (offset + length !== text.length) return { text, done: [], doneLength: 0, tail: [...done, ...tokens], doneLinks: {}, incremental: false }
  let doneLength = offset
  for (let at = 0; at < cut; at += 1) doneLength += tokens[at].raw.length
  const settled = tokens.slice(0, cut)
  return {
    text, done: cut > 0 ? [...done, ...settled] : done, doneLength, tail: tokens.slice(cut),
    doneLinks: cut > 0 ? collectLinks(settled, { ...doneLinks }) : doneLinks, incremental: true,
  }
}

/**
 * Lexes `text`, reusing `previous` when the text only grew: the tokens before the open tail
 * (`openFrom`) are final, so only the source from the tail on is lexed again.
 * A reference definition in that source, or text that did not just grow, means a full lex. The result
 * is the same as a full lex of `text`.
 */
export function lexMarkdown(text: string, previous: LexState | null = null): LexState {
  const source = normalize(text)
  if (previous?.incremental && source.length >= previous.text.length && source.startsWith(previous.text)) {
    if (source === previous.text) return previous
    const lexer = newLexer(previous.doneLinks)
    const tokens = lexer.lex(source.slice(previous.doneLength))
    // A new definition may change links in finished blocks: read everything again.
    if (!hasDef(tokens) && Object.keys(lexer.tokens.links).length === Object.keys(previous.doneLinks).length) {
      return settle(source, previous.done, previous.doneLinks, previous.doneLength, tokens)
    }
  }
  return settle(source, [], {}, 0, newLexer().lex(source))
}

/**
 * The view-facing pieces of a lexed text. With `open` (a reply still streaming), the tail is read
 * again after `closeOpenMarkdown`, so an unfinished `**bold` shows as bold rather than as stars.
 */
export function markdownPieces(state: LexState, open: boolean): MarkdownPiece[] {
  const visible = (token: Token) => token.type !== 'space' && token.type !== 'def'
  const pieces: MarkdownPiece[] = state.done.filter(visible).map((token) => ({ raw: token.raw, token, open: false }))
  if (!open || state.tail.length === 0) {
    for (const token of state.tail) if (visible(token)) pieces.push({ raw: token.raw, token, open })
    return pieces
  }
  const source = state.text.slice(state.incremental ? state.doneLength : 0)
  for (const token of newLexer(state.doneLinks).lex(closeOpenMarkdown(source))) if (visible(token)) pieces.push({ raw: token.raw, token, open: true })
  return pieces
}

/**
 * A reader that keeps its state between calls, for a view that is handed a growing text. The same
 * text twice (React's double render) returns the same pieces.
 */
export function createMarkdownReader() {
  let state: LexState | null = null
  let last: { text: string; open: boolean; pieces: MarkdownPiece[] } | null = null
  return (text: string, open: boolean): MarkdownPiece[] => {
    if (last && last.text === text && last.open === open) return last.pieces
    state = lexMarkdown(text, state)
    const pieces = markdownPieces(state, open)
    // Finished pieces keep their identity from one update to the next.
    if (last) for (let at = 0; at < pieces.length && at < last.pieces.length; at += 1) {
      const before = last.pieces[at]
      if (before.raw === pieces[at].raw && before.open === pieces[at].open && !before.open) pieces[at] = before
    }
    last = { text, open, pieces }
    return pieces
  }
}

/** Parses the whole text. `open` closes what a streaming reply has left open. */
export function parseMarkdown(text: string, open = false): MarkdownBlock[] {
  return markdownPieces(lexMarkdown(text), open).flatMap((piece) => blocksOf(piece.token))
}

/** Inline text only. `open` closes unfinished bold, italic, code and links at the end of the text. */
export function parseInline(text: string, open = false): Inline[] {
  return inlines(newLexer().inlineTokens(normalize(open ? closeOpenMarkdown(text) : text)))
}

// --- Closing what streaming left open ---------------------------------------------------------

const FENCE_OPEN = /^\s*(`{3,}|~{3,})/

/** Cells in a table row, `| a | b |` or `a | b`. */
const cellCount = (line: string) => {
  const cells = line.trim().replace(/^\|/, '').replace(/(?<!\\)\|$/, '').split(/(?<!\\)\|/)
  return cells.length
}
const TABLE_ROW = /^\s{0,3}\|.*\|/
const DELIMITER_PART = /^\s{0,3}\|?[\s:|-]*$/
const completeDelimiter = (line: string, count: number) => {
  const cells = line.trim().replace(/^\|/, '').replace(/\|$/, '').split('|')
  return cells.length === count && cells.every((cell) => /^\s*:?-+:?\s*$/.test(cell))
}

/**
 * Closes, at the end of a reply that is still arriving, what would otherwise flicker between raw
 * markers and formatting: emphasis (`**`, `*`, `_`, `~~`) and inline code on the last line are closed,
 * a marker with nothing after it yet is dropped, a half-written `[label](addr` shows just its label,
 * and a table whose header has arrived gets its delimiter row early. An unclosed code fence is left
 * alone: it already reads as code to the end.
 */
export function closeOpenMarkdown(text: string): string {
  const source = normalize(text)
  const lines = source.split('\n')
  let fence: string | null = null
  for (const line of lines) {
    if (fence !== null) {
      if (new RegExp(`^\\s*${fence[0] === '`' ? '`' : '~'}{${fence.length},}\\s*$`).test(line)) fence = null
      continue
    }
    const open = FENCE_OPEN.exec(line)
    if (open) fence = open[1]
  }
  if (fence !== null) return source

  const head = lines.slice(0, -1)
  let line = lines[lines.length - 1]
  const previous = head[head.length - 1]

  // A table: a header row gets its delimiter row early, and a delimiter row still being typed is completed.
  const header = (row: string | undefined, before: string | undefined) =>
    row !== undefined && TABLE_ROW.test(row) && cellCount(row) >= 2 && !DELIMITER_PART.test(row) && (before === undefined || !TABLE_ROW.test(before))
  const delimiter = (count: number) => `${'|---'.repeat(count)}|`
  if (header(line, previous)) return `${source}\n${delimiter(cellCount(line))}`
  if (header(previous, head[head.length - 2]) && DELIMITER_PART.test(line) && !completeDelimiter(line, cellCount(previous!))) {
    return [...head, delimiter(cellCount(previous!))].join('\n')
  }

  // A half-written link or image shows its label until the address is complete.
  const link = /(!?)\[([^[\]\n]*)(\]\([^()\s]*)?$/.exec(line)
  if (link && line[link.index - 1] !== '\\' && evenTicks(line.slice(0, link.index)) && (link[3] !== undefined || link[2] !== '')) {
    line = line.slice(0, link.index) + link[2]
  }
  // A marker with nothing after it yet.
  line = line.replace(/(^|[\s(])(\*{1,3}|_{1,3}|~~|`+)$/, '$1')

  const trailing = /\s*$/.exec(line)![0]
  const body = line.slice(0, line.length - trailing.length)
  const closers = openMarkers(body)
  return [...head, body + closers + trailing].join('\n')
}

const evenTicks = (text: string) => ((text.match(/`/g) ?? []).length % 2) === 0

/** The closers, innermost first, for the emphasis and code that `line` leaves open. */
function openMarkers(line: string): string {
  const stack: string[] = []
  let at = 0
  const word = (char: string | undefined) => char !== undefined && /[\p{L}\p{N}]/u.test(char)
  const space = (char: string | undefined) => char === undefined || /\s/.test(char)
  while (at < line.length) {
    const char = line[at]
    if (char === '\\') { at += 2; continue }
    if (char === '`') {
      const run = /^`+/.exec(line.slice(at))![0]
      const end = line.indexOf(run, at + run.length)
      if (end < 0) return run + stack.reverse().join('')
      at = end + run.length
      continue
    }
    if (char === '*' || char === '_' || char === '~') {
      const run = new RegExp(`^\\${char}+`).exec(line.slice(at))![0]
      const before = line[at - 1]
      const after = line[at + run.length]
      at += run.length
      // A bullet, a `~` that is not `~~`, or a run too long to be emphasis.
      if ((char === '*' && run.length === 1 && at === 1 + (line.length - line.trimStart().length) && after === ' ') || (char === '~' && run.length !== 2) || run.length > 3) continue
      const canOpen = !space(after) && (char !== '_' || !word(before))
      const canClose = !space(before) && (char !== '_' || !word(after))
      const index = stack.lastIndexOf(run)
      if (canClose && index >= 0) stack.length = index
      else if (canOpen) stack.push(run)
      continue
    }
    at += 1
  }
  return stack.reverse().join('')
}

// --- Tokens to data ---------------------------------------------------------------------------

const textOf = (text: string): Inline => ({ type: 'text', text })

/** Inline tokens as data. Adjacent text is joined into one node. */
export function inlines(tokens: Token[]): Inline[] {
  const out: Inline[] = []
  const push = (node: Inline) => {
    const last = out[out.length - 1]
    if (node.type === 'text' && last?.type === 'text') out[out.length - 1] = textOf(last.text + node.text)
    else if (node.type !== 'text' || node.text !== '') out.push(node)
  }
  for (const token of tokens) {
    switch (token.type) {
      case 'text': {
        const text = token as Tokens.Text
        if (text.tokens && text.tokens.length > 0) for (const node of inlines(text.tokens)) push(node)
        else push(textOf(decodeEntities(text.text)))
        break
      }
      case 'escape': push(textOf((token as Tokens.Escape).text)); break
      case 'br': push({ type: 'br' }); break
      case 'codespan': push({ type: 'code', text: (token as Tokens.Codespan).text }); break
      case 'strong': push({ type: 'strong', children: inlines((token as Tokens.Strong).tokens) }); break
      case 'em': push({ type: 'em', children: inlines((token as Tokens.Em).tokens) }); break
      case 'del': push({ type: 'del', children: inlines((token as Tokens.Del).tokens) }); break
      // Images are never loaded: only the description stays.
      case 'image': push(textOf(decodeEntities((token as Tokens.Image).text))); break
      // Raw HTML is shown as written.
      case 'html': case 'tag': push(textOf(token.raw)); break
      case 'checkbox': break
      case 'link': {
        const link = token as Tokens.Link
        const href = decodeEntities(link.href)
        const ref = parseRef(href, decodeEntities(link.text))
        const safe = ref ? null : safeHref(href)
        if (ref) push(ref)
        else if (safe) push({ type: 'link', href: safe, children: inlines(link.tokens) })
        // An autolink to an address the app will not open stays as written; a labelled one keeps its label.
        else if (link.raw.startsWith('<')) push(textOf(link.raw))
        else for (const node of inlines(link.tokens)) push(node)
        break
      }
      default: push(textOf(token.raw))
    }
  }
  return out
}

/**
 * A paragraph as blocks: a line holding nothing but one reference becomes a card of its own, the
 * lines around it stay paragraphs.
 */
function paragraphBlocks(children: Inline[]): MarkdownBlock[] {
  if (!children.some((node) => node.type === 'ref')) return [{ type: 'paragraph', children }]
  const blocks: MarkdownBlock[] = []
  let lines: Inline[][] = []
  const flush = () => {
    if (lines.length > 0) blocks.push({ type: 'paragraph', children: lines.flatMap((line, at) => at === 0 ? line : [{ type: 'br' } as Inline, ...line]) })
    lines = []
  }
  let line: Inline[] = []
  const endLine = () => {
    const content = line.filter((node) => node.type !== 'text' || node.text.trim() !== '')
    if (content.length === 1 && content[0].type === 'ref') { flush(); blocks.push(content[0]) } else lines.push(line)
    line = []
  }
  for (const node of children) {
    if (node.type === 'br') endLine()
    else line.push(node)
  }
  endLine()
  flush()
  return blocks
}

const ALIGN = new Set(['left', 'center', 'right'])

/** One block token as data: none for spacing and definitions, several when a reference splits a paragraph. */
export function blocksOf(token: Token): MarkdownBlock[] {
  switch (token.type) {
    case 'space': case 'def': return []
    case 'paragraph': return paragraphBlocks(inlines((token as Tokens.Paragraph).tokens))
    case 'text': {
      const text = token as Tokens.Text
      return paragraphBlocks(text.tokens ? inlines(text.tokens) : [textOf(decodeEntities(text.text))])
    }
    case 'heading': {
      const heading = token as Tokens.Heading
      return [{ type: 'heading', level: Math.min(Math.max(heading.depth, 1), 6) as HeadingLevel, children: inlines(heading.tokens) }]
    }
    case 'code': {
      const code = token as Tokens.Code
      return [{ type: 'code', lang: code.lang?.trim().split(/\s+/)[0] || null, text: code.text }]
    }
    case 'blockquote': return [{ type: 'quote', children: (token as Tokens.Blockquote).tokens.flatMap(blocksOf) }]
    case 'list': {
      const list = token as Tokens.List
      const start = typeof list.start === 'number' ? list.start : Number.parseInt(String(list.start), 10)
      return [{
        type: 'list', ordered: list.ordered, start: Number.isFinite(start) ? start : 1, loose: list.loose,
        items: list.items.map((item) => ({ checked: item.task ? item.checked === true : null, blocks: item.tokens.flatMap(blocksOf) })),
      }]
    }
    case 'table': {
      const table = token as Tokens.Table
      return [{
        type: 'table',
        align: table.align.map((align) => (align && ALIGN.has(align) ? align : null)),
        header: table.header.map((cell) => inlines(cell.tokens)),
        rows: table.rows.map((row) => row.map((cell) => inlines(cell.tokens))),
      }]
    }
    case 'hr': return [{ type: 'rule' }]
    case 'checkbox': return []
    case 'html': {
      // Raw HTML is shown as written, line by line.
      const lines = token.raw.replace(/\n+$/, '').split('\n')
      return [{ type: 'paragraph', children: lines.flatMap((line, at): Inline[] => at === 0 ? [textOf(line)] : [{ type: 'br' }, textOf(line)]) }]
    }
    default: return token.raw.trim() === '' ? [] : [{ type: 'paragraph', children: [textOf(token.raw.trim())] }]
  }
}
