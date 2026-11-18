import { describe, expect, it } from 'vitest'
import { closeOpenMarkdown, createMarkdownReader, decodeEntities, lexMarkdown, markdownPieces, parseInline, parseMarkdown, safeHref, blocksOf, type LexState } from './markdown'

describe('safeHref', () => {
  it('allows http, https and mailto only', () => {
    expect(safeHref('https://example.com/a?b=1')).toBe('https://example.com/a?b=1')
    expect(safeHref('http://example.com')).toBe('http://example.com/')
    expect(safeHref('mailto:a@example.com')).toBe('mailto:a@example.com')
    for (const bad of ['javascript:alert(1)', 'JaVaScRiPt:alert(1)', 'data:text/html,hi', 'file:///etc/passwd', 'tauri://localhost', '/relative', 'example.com', 'https://', 'mailto:', 'java\nscript:alert(1)']) {
      expect(safeHref(bad), bad).toBeNull()
    }
  })
})

describe('inline', () => {
  it('reads bold, italic, code and line breaks', () => {
    expect(parseInline('a **b** *c* `d`\ne')).toEqual([
      { type: 'text', text: 'a ' }, { type: 'strong', children: [{ type: 'text', text: 'b' }] }, { type: 'text', text: ' ' },
      { type: 'em', children: [{ type: 'text', text: 'c' }] }, { type: 'text', text: ' ' }, { type: 'code', text: 'd' },
      { type: 'br' }, { type: 'text', text: 'e' },
    ])
  })

  it('shows HTML as text', () => {
    expect(parseInline('<script>alert(1)</script> <b>x</b>')).toEqual([{ type: 'text', text: '<script>alert(1)</script> <b>x</b>' }])
    expect(parseMarkdown('<img src=x onerror=alert(1)>')).toEqual([{ type: 'paragraph', children: [{ type: 'text', text: '<img src=x onerror=alert(1)>' }] }])
  })

  it('keeps safe links and drops the address of unsafe ones', () => {
    expect(parseInline('[docs](https://example.com/x)')).toEqual([{ type: 'link', href: 'https://example.com/x', children: [{ type: 'text', text: 'docs' }] }])
    expect(parseInline('[bad](javascript:alert(1))')).toEqual([{ type: 'text', text: 'bad' }])
    expect(parseInline('[bad](data:text/html;base64,AAAA)')).toEqual([{ type: 'text', text: 'bad' }])
    expect(parseInline('<javascript:alert(1)>')).toEqual([{ type: 'text', text: '<javascript:alert(1)>' }])
  })

  it('links bare addresses without their trailing punctuation', () => {
    expect(parseInline('see https://example.com/a.')).toEqual([
      { type: 'text', text: 'see ' }, { type: 'link', href: 'https://example.com/a', children: [{ type: 'text', text: 'https://example.com/a' }] }, { type: 'text', text: '.' },
    ])
  })

  it('never loads images: only the description stays', () => {
    expect(parseInline('![logo](https://example.com/x.png) hi')).toEqual([{ type: 'text', text: 'logo hi' }])
  })

  it('leaves an unclosed construct literal, and closes it while streaming', () => {
    expect(parseInline('a **bol')).toEqual([{ type: 'text', text: 'a **bol' }])
    expect(parseInline('a **bol', true)).toEqual([{ type: 'text', text: 'a ' }, { type: 'strong', children: [{ type: 'text', text: 'bol' }] }])
    expect(parseInline('use `cod', true)).toEqual([{ type: 'text', text: 'use ' }, { type: 'code', text: 'cod' }])
    // A half-written link shows its label as plain text until the address is complete; it never becomes a link early.
    expect(parseInline('[docs](', true)).toEqual([{ type: 'text', text: 'docs' }])
    expect(parseInline('[docs](https://exa', true).some((node) => node.type === 'link' && node.children[0]?.type === 'text' && node.children[0].text === 'docs')).toBe(false)
  })

  it('does not treat spaced stars or snake_case as emphasis', () => {
    expect(parseInline('2 * 3 * 4')).toEqual([{ type: 'text', text: '2 * 3 * 4' }])
    expect(parseInline('snake_case_name')).toEqual([{ type: 'text', text: 'snake_case_name' }])
  })

  it('reads strikethrough', () => {
    expect(parseInline('~~old~~ new')).toEqual([{ type: 'del', children: [{ type: 'text', text: 'old' }] }, { type: 'text', text: ' new' }])
  })

  it('decodes HTML entities in text but not in code', () => {
    expect(parseInline('Q&amp;A &lt;tag&gt; &#39;x&#x27; &copy; &nope; `&amp;`')).toEqual([
      { type: 'text', text: 'Q&A <tag> \'x\' © &nope; ' }, { type: 'code', text: '&amp;' },
    ])
    expect(decodeEntities('&#0;&#xD800;&#128512;')).toBe('\uFFFD\uFFFD😀')
  })

  it('ends a bare address at CJK punctuation', () => {
    for (const mark of ['。', '，', '；', '：', '！', '？', '）', '】', '」', '』']) {
      expect(parseInline(`见 https://example.com/a${mark}后面`), mark).toEqual([
        { type: 'text', text: '见 ' }, { type: 'link', href: 'https://example.com/a', children: [{ type: 'text', text: 'https://example.com/a' }] }, { type: 'text', text: `${mark}后面` },
      ])
    }
    expect(parseInline('(see https://example.com/x)')).toEqual([
      { type: 'text', text: '(see ' }, { type: 'link', href: 'https://example.com/x', children: [{ type: 'text', text: 'https://example.com/x' }] }, { type: 'text', text: ')' },
    ])
    expect(parseInline('<https://example.com/a>')).toEqual([{ type: 'link', href: 'https://example.com/a', children: [{ type: 'text', text: 'https://example.com/a' }] }])
  })

  it('takes a backslash escape literally', () => {
    expect(parseInline('\\*not\\*')).toEqual([{ type: 'text', text: '*not*' }])
  })
})

describe('blocks', () => {
  it('reads an unfinished fence as code to the end', () => {
    expect(parseMarkdown('```\nlet a\n\nlet b')).toEqual([{ type: 'code', lang: null, text: 'let a\n\nlet b' }])
    expect(parseMarkdown('intro\n\n```ts\nlet a', true)).toEqual([
      { type: 'paragraph', children: [{ type: 'text', text: 'intro' }] },
      { type: 'code', lang: 'ts', text: 'let a' },
    ])
  })

  it('reads bulleted and numbered lists, with their start and nesting', () => {
    expect(parseMarkdown('- one\n- two\n  - inner')).toEqual([{
      type: 'list', ordered: false, start: 1, loose: false,
      items: [
        { checked: null, blocks: [{ type: 'paragraph', children: [{ type: 'text', text: 'one' }] }] },
        { checked: null, blocks: [
          { type: 'paragraph', children: [{ type: 'text', text: 'two' }] },
          { type: 'list', ordered: false, start: 1, loose: false, items: [{ checked: null, blocks: [{ type: 'paragraph', children: [{ type: 'text', text: 'inner' }] }] }] },
        ] },
      ],
    }])
    // Items apart by a blank line are still one list.
    expect(parseMarkdown('1. a\n\n2. b')).toMatchObject([{ type: 'list', ordered: true, start: 1, loose: true, items: [{}, {}] }])
    expect(parseMarkdown('3) c\n4) d')).toMatchObject([{ type: 'list', ordered: true, start: 3 }])
  })

  it('keeps a loose list together with its continuation paragraphs, nested bullets and code', () => {
    const text = '1. First step\n\n   More about it.\n\n2. Second step\n   - a\n   - b\n\n   ```sh\n   run it\n   ```\n\n3. Third'
    const [list] = parseMarkdown(text)
    expect(list).toMatchObject({ type: 'list', ordered: true, start: 1, loose: true })
    if (list.type !== 'list') throw new Error('not a list')
    expect(list.items).toHaveLength(3)
    expect(list.items[0].blocks).toEqual([
      { type: 'paragraph', children: [{ type: 'text', text: 'First step' }] },
      { type: 'paragraph', children: [{ type: 'text', text: 'More about it.' }] },
    ])
    expect(list.items[1].blocks.map((block) => block.type)).toEqual(['paragraph', 'list', 'code'])
    expect(list.items[1].blocks[2]).toEqual({ type: 'code', lang: 'sh', text: 'run it' })
  })

  it('reads task items as checked or not, without a box to tick', () => {
    expect(parseMarkdown('- [x] done\n- [ ] to do\n- plain')).toEqual([{
      type: 'list', ordered: false, start: 1, loose: false,
      items: [
        { checked: true, blocks: [{ type: 'paragraph', children: [{ type: 'text', text: 'done' }] }] },
        { checked: false, blocks: [{ type: 'paragraph', children: [{ type: 'text', text: 'to do' }] }] },
        { checked: null, blocks: [{ type: 'paragraph', children: [{ type: 'text', text: 'plain' }] }] },
      ],
    }])
  })

  it('reads a blockquote and headings with their level', () => {
    expect(parseMarkdown('> quoted\n> more')).toEqual([{ type: 'quote', children: [{ type: 'paragraph', children: [{ type: 'text', text: 'quoted' }, { type: 'br' }, { type: 'text', text: 'more' }] }] }])
    expect(parseMarkdown('## Title')).toEqual([{ type: 'heading', level: 2, children: [{ type: 'text', text: 'Title' }] }])
    expect(parseMarkdown('# One\n\n### Three **bold**\n\n###### Six')).toEqual([
      { type: 'heading', level: 1, children: [{ type: 'text', text: 'One' }] },
      { type: 'heading', level: 3, children: [{ type: 'text', text: 'Three ' }, { type: 'strong', children: [{ type: 'text', text: 'bold' }] }] },
      { type: 'heading', level: 6, children: [{ type: 'text', text: 'Six' }] },
    ])
  })

  it('reads a horizontal rule', () => {
    expect(parseMarkdown('a\n\n---\n\nb')).toEqual([
      { type: 'paragraph', children: [{ type: 'text', text: 'a' }] }, { type: 'rule' }, { type: 'paragraph', children: [{ type: 'text', text: 'b' }] },
    ])
  })

  it('reads a fenced code block with its language and no highlighting', () => {
    expect(parseMarkdown('```rust title="x"\nfn main() {}\n```')).toEqual([{ type: 'code', lang: 'rust', text: 'fn main() {}' }])
    expect(parseMarkdown('~~~\n<b>x</b> &amp;\n~~~')).toEqual([{ type: 'code', lang: null, text: '<b>x</b> &amp;' }])
  })

  it('reads a table with its alignment and inline content', () => {
    expect(parseMarkdown('| Name | Count | Note |\n|:--|--:|:-:|\n| **a** | 12 | `x` |\n| [b](https://example.com) | 3 | ~~old~~ |')).toEqual([{
      type: 'table',
      align: ['left', 'right', 'center'],
      header: [[{ type: 'text', text: 'Name' }], [{ type: 'text', text: 'Count' }], [{ type: 'text', text: 'Note' }]],
      rows: [
        [[{ type: 'strong', children: [{ type: 'text', text: 'a' }] }], [{ type: 'text', text: '12' }], [{ type: 'code', text: 'x' }]],
        [[{ type: 'link', href: 'https://example.com/', children: [{ type: 'text', text: 'b' }] }], [{ type: 'text', text: '3' }], [{ type: 'del', children: [{ type: 'text', text: 'old' }] }]],
      ],
    }])
    expect(parseMarkdown('| a | b |\n|---|---|\n| 1 | 2 |')).toMatchObject([{ type: 'table', align: [null, null] }])
  })

  it('sanitizes table cells like any other text', () => {
    const [table] = parseMarkdown('| a | b |\n|---|---|\n| [x](javascript:alert(1)) | <img src=x onerror=alert(1)> |\n| ![p](https://e.com/p.png) | [m](mailx://email/abc) |')
    expect(table).toEqual({
      type: 'table', align: [null, null],
      header: [[{ type: 'text', text: 'a' }], [{ type: 'text', text: 'b' }]],
      rows: [
        [[{ type: 'text', text: 'x' }], [{ type: 'text', text: '<img src=x onerror=alert(1)>' }]],
        [[{ type: 'text', text: 'p' }], [{ type: 'ref', kind: 'email', id: 'abc', label: 'm' }]],
      ],
    })
  })

  it('shows block HTML as text', () => {
    expect(parseMarkdown('<div onclick="x()">\nhi\n</div>')).toEqual([{ type: 'paragraph', children: [{ type: 'text', text: '<div onclick="x()">' }, { type: 'br' }, { type: 'text', text: 'hi' }, { type: 'br' }, { type: 'text', text: '</div>' }] }])
    expect(parseMarkdown('<script>alert(1)</script>')).toEqual([{ type: 'paragraph', children: [{ type: 'text', text: '<script>alert(1)</script>' }] }])
  })

  it('resolves reference-style links, only to safe addresses', () => {
    expect(parseMarkdown('See [the docs][d] and [bad][b].\n\n[d]: https://example.com/docs\n[b]: javascript:alert(1)')).toEqual([{
      type: 'paragraph',
      children: [
        { type: 'text', text: 'See ' }, { type: 'link', href: 'https://example.com/docs', children: [{ type: 'text', text: 'the docs' }] },
        { type: 'text', text: ' and bad.' },
      ],
    }])
  })

  it('treats a streamed half list item as a list, and only the last block as open', () => {
    expect(parseMarkdown('intro\n\n- a\n- **b', true)).toMatchObject([
      { type: 'paragraph' },
      { type: 'list', items: [{ blocks: [{ type: 'paragraph' }] }, { blocks: [{ type: 'paragraph', children: [{ type: 'strong' }] }] }] },
    ])
    // Only the end is closed: an earlier block keeps its stars.
    expect(parseMarkdown('a **b\n\nc *d', true)).toEqual([
      { type: 'paragraph', children: [{ type: 'text', text: 'a **b' }] },
      { type: 'paragraph', children: [{ type: 'text', text: 'c ' }, { type: 'em', children: [{ type: 'text', text: 'd' }] }] },
    ])
  })

  it('does not throw on partial input at any length', () => {
    for (let at = 0; at <= SAMPLE.length; at += 1) expect(() => parseMarkdown(SAMPLE.slice(0, at), true)).not.toThrow()
  })
})

describe('closing what streaming left open', () => {
  it('closes emphasis, strikethrough and code on the last line, innermost first', () => {
    expect(closeOpenMarkdown('a **bol')).toBe('a **bol**')
    expect(closeOpenMarkdown('a **b *c')).toBe('a **b *c***')
    expect(closeOpenMarkdown('x _it')).toBe('x _it_')
    expect(closeOpenMarkdown('so ~~old')).toBe('so ~~old~~')
    expect(closeOpenMarkdown('run `ls -')).toBe('run `ls -`')
    expect(closeOpenMarkdown('**a `b')).toBe('**a `b`**')
    expect(closeOpenMarkdown('a **b** done')).toBe('a **b** done')
  })

  it('drops a marker with nothing after it yet, and leaves what is not emphasis alone', () => {
    expect(closeOpenMarkdown('a **')).toBe('a ')
    expect(closeOpenMarkdown('say `')).toBe('say ')
    expect(closeOpenMarkdown('2 * 3 * 4')).toBe('2 * 3 * 4')
    expect(closeOpenMarkdown('snake_case_na')).toBe('snake_case_na')
    expect(closeOpenMarkdown('* item *ok')).toBe('* item *ok*')
    expect(closeOpenMarkdown('about ~5')).toBe('about ~5')
  })

  it('shows the label of a half-written link and never the half address', () => {
    expect(closeOpenMarkdown('see [docs](https://exa')).toBe('see docs')
    expect(closeOpenMarkdown('see [do')).toBe('see do')
    expect(closeOpenMarkdown('see ![logo](https://')).toBe('see logo')
    expect(closeOpenMarkdown('see [docs](https://example.com)')).toBe('see [docs](https://example.com)')
    expect(closeOpenMarkdown('- [ ] task')).toBe('- [ ] task')
  })

  it('gives a table its delimiter row as soon as the header is there', () => {
    expect(closeOpenMarkdown('| a | b |')).toBe('| a | b |\n|---|---|')
    expect(closeOpenMarkdown('| a | b |\n')).toBe('| a | b |\n|---|---|')
    expect(closeOpenMarkdown('| a | b |\n|:-')).toBe('| a | b |\n|---|---|')
    expect(closeOpenMarkdown('| a | b |\n|:-|-:|')).toBe('| a | b |\n|:-|-:|')
    expect(closeOpenMarkdown('| a | b |\n|---|---|\n| 1 | **2')).toBe('| a | b |\n|---|---|\n| 1 | **2**')
  })

  it('leaves an unclosed code fence alone', () => {
    expect(closeOpenMarkdown('```js\nconst a = **b')).toBe('```js\nconst a = **b')
    expect(closeOpenMarkdown('```\nx\n```\nnow **b')).toBe('```\nx\n```\nnow **b**')
  })

  it('shows a table, a fence, bold and a link sensibly at every cut', () => {
    const table = '| Item | Due |\n|:--|--:|\n| Bill | Fri |\n| Talk | Mon |'
    for (let at = '| Item | D'.length; at <= table.length; at += 1) {
      const blocks = parseMarkdown(table.slice(0, at), true)
      expect(blocks.map((block) => block.type), table.slice(0, at)).toEqual(['table'])
    }
    const fence = '```ts\nconst a = 1\n```'
    for (let at = 3; at <= fence.length; at += 1) {
      expect(parseMarkdown(fence.slice(0, at), true).map((block) => block.type), fence.slice(0, at)).toEqual(['code'])
    }
    const bold = 'Pay **the bill** today'
    for (let at = 0; at <= bold.length; at += 1) {
      const text = JSON.stringify(parseMarkdown(bold.slice(0, at), true))
      expect(text.includes('**'), bold.slice(0, at)).toBe(false)
    }
    const link = 'Read [the docs](https://example.com/docs) now'
    for (let at = 0; at <= link.length; at += 1) {
      const text = JSON.stringify(parseMarkdown(link.slice(0, at), true))
      expect(text.includes('](') || text.includes('https://exa') && !text.includes('"link"'), link.slice(0, at)).toBe(false)
    }
  })
})

/** A reply that uses most of what the assistant can write. */
const SAMPLE = [
  '# Weekly summary', '', 'Hi **there**, here is what _needs_ you:', '',
  '1. Pay the bill', '', '   It is due on **Friday**.', '', '2. Reply to the team', '   - check `dates`', '   - [x] read the brief', '   - [ ] write back', '',
  '| Item | Due | Amount |', '|:--|:-:|--:|', '| Bill | Fri | 1,200 |', '| Talk | Mon | 0 |', '',
  '> A quote with a [link](https://example.com/a) and https://example.com/b。', '',
  '```ts', 'const a = 1', '', 'const b = 2', '```', '', '---', '',
  'Old ~~plan~~, see [mail](mailx://email/abc-123) and &amp; [ref][r].', '',
  '[x](mailx://folder/acc:inbox)', '', '[r]: https://example.com/r', '', 'Done <b>bye</b>.',
].join('\n')

describe('incremental lexing', () => {
  const blocks = (state: LexState, open: boolean) => markdownPieces(state, open).flatMap((piece) => blocksOf(piece.token))

  it('gives the same result as a full lex for every prefix of a reply', () => {
    let state: LexState | null = null
    for (let at = 0; at <= SAMPLE.length; at += 1) {
      const text = SAMPLE.slice(0, at)
      state = lexMarkdown(text, state)
      const full = lexMarkdown(text)
      expect(state.done.length + state.tail.length >= 0).toBe(true)
      expect(blocks(state, false), text).toEqual(blocks(full, false))
      expect(blocks(state, true), text).toEqual(blocks(full, true))
    }
  })

  it('gives the same result when the text grows by uneven chunks', () => {
    for (const step of [3, 7, 16, 41]) {
      let state: LexState | null = null
      for (let at = 0; at < SAMPLE.length + step; at += step) {
        const text = SAMPLE.slice(0, at)
        state = lexMarkdown(text, state)
        expect(blocks(state, true), `${step}:${at}`).toEqual(blocks(lexMarkdown(text), true))
      }
    }
  })

  it('lexes only the tail again while the text grows, and starts over when it does not', () => {
    const first = lexMarkdown('para one\n\npara two')
    expect(first.done.map((token) => token.type)).toEqual(['paragraph', 'space'])
    expect(first.doneLength).toBe('para one\n\n'.length)
    const second = lexMarkdown('para one\n\npara two\n\n- a', first)
    // The finished paragraph is the very same token: it was not lexed again.
    expect(second.done[0]).toBe(first.done[0])
    const changed = lexMarkdown('other text', second)
    expect(changed.done).toHaveLength(0)
    expect(changed.tail.map((token) => token.type)).toEqual(['paragraph'])
  })

  it('keeps finished pieces as they were and marks only the tail open', () => {
    const read = createMarkdownReader()
    const one = read('a\n\nb **c', true)
    expect(one.map((piece) => piece.open)).toEqual([false, true])
    const two = read('a\n\nb **c** d\n\ne', true)
    expect(two[0]).toBe(one[0])
    expect(two.map((piece) => piece.open)).toEqual([false, false, true])
    expect(read('a\n\nb **c** d\n\ne', true)).toBe(two)
    expect(read('a\n\nb **c** d\n\ne', false).map((piece) => piece.open)).toEqual([false, false, false])
  })
})
describe('mailx references', () => {
  const UUID = '3f2b8c1e-5d4a-4c7b-9e10-aa11bb22cc33'

  it('makes a card of a reference alone on its line, and a chip inside a sentence', () => {
    expect(parseMarkdown(`[账单](mailx://email/${UUID})`)).toEqual([{ type: 'ref', kind: 'email', id: UUID, label: '账单' }])
    expect(parseMarkdown(`看这里：\n[收件箱](mailx://folder/${UUID})\n然后呢`)).toEqual([
      { type: 'paragraph', children: [{ type: 'text', text: '看这里：' }] },
      { type: 'ref', kind: 'folder', id: UUID, label: '收件箱' },
      { type: 'paragraph', children: [{ type: 'text', text: '然后呢' }] },
    ])
    expect(parseMarkdown(`请先看[这封](mailx://email/${UUID})。`)).toEqual([{
      type: 'paragraph',
      children: [{ type: 'text', text: '请先看' }, { type: 'ref', kind: 'email', id: UUID, label: '这封' }, { type: 'text', text: '。' }],
    }])
  })

  it('reads folder ids that carry a path', () => {
    expect(parseInline('[x](mailx://folder/acc:projects/mailx)')).toEqual([{ type: 'ref', kind: 'folder', id: 'acc:projects/mailx', label: 'x' }])
  })

  it('keeps any other mailx address, or a malformed id, as plain text', () => {
    for (const bad of ['mailx://email/', 'mailx://email/../x', 'mailx://email/a/b', 'mailx://folder/', 'mailx://folder/-x', 'mailx://folder/a?b=1', 'mailx://other/1', 'mailx://draft/abc', 'mailx:email/abc', `mailx://email/${'a'.repeat(129)}`]) {
      expect(parseInline(`[label](${bad})`), bad).toEqual([{ type: 'text', text: 'label' }])
      expect(parseMarkdown(`[label](${bad})`), bad).toEqual([{ type: 'paragraph', children: [{ type: 'text', text: 'label' }] }])
    }
  })

  it('keeps an address with a space as the text it is: it is not a link at all', () => {
    expect(parseInline('[label](mailx://email/a b)')).toEqual([{ type: 'text', text: '[label](mailx://email/a b)' }])
  })

  it('leaves http links alone and never turns a reference into a link', () => {
    expect(parseInline('[a](https://example.com)')).toEqual([{ type: 'link', href: 'https://example.com/', children: [{ type: 'text', text: 'a' }] }])
    expect(parseInline(`[a](mailx://email/${UUID})`)).toEqual([{ type: 'ref', kind: 'email', id: UUID, label: 'a' }])
  })

  it('stays text while the address is still arriving', () => {
    for (const partial of ['[x](mailx://ema', '[x](mailx://email/3f2b8c1e', '[x](mailx:/']) {
      expect(() => parseMarkdown(partial, true), partial).not.toThrow()
      expect(parseMarkdown(partial, true).every((block) => block.type === 'paragraph'), partial).toBe(true)
    }
    // Only the label shows until the address is complete.
    expect(parseMarkdown('[x](mailx://ema', true)).toEqual([{ type: 'paragraph', children: [{ type: 'text', text: 'x' }] }])
  })
})
