import { describe, expect, it } from 'vitest'
import { NOTES_LIMIT, parseInline, parseNotes, safeLink, truncateNotes } from './notes-markdown'

const text = (notes: string) => parseNotes(notes)

describe('release notes rendering', () => {
  it('shows raw HTML as text, never as markup', () => {
    const blocks = text('<img src=x onerror=alert(1)> and <b>bold</b>')
    expect(blocks).toHaveLength(1)
    const inline = blocks[0].kind === 'paragraph' ? blocks[0].children : []
    expect(inline.map((node) => (node.kind === 'text' ? node.text : node.kind))).toEqual([
      '<img src=x onerror=alert(1)> and <b>bold</b>',
    ])
    expect(JSON.stringify(blocks)).not.toContain('dangerouslySetInnerHTML')
  })

  it('keeps links to other schemes as text, without a link node', () => {
    for (const bad of [
      'javascript:alert(1)',
      'JavaScript:alert(1)',
      'data:text/html,<script>alert(1)</script>',
      'file:///etc/passwd',
      'mailto:a@b.c',
      'https:/one-slash',
      'https://',
      ' //example.com',
    ]) {
      expect(safeLink(bad)).toBeNull()
      const inline = parseInline(`[click](${bad})`)
      expect(inline.some((node) => node.kind === 'link'), bad).toBe(false)
      expect(inline.every((node) => node.kind !== 'link' || node.url.startsWith('https://')), bad).toBe(true)
    }
    expect(safeLink('https://github.com/GuqierMcl/MailX/releases/tag/v0.2.0')).not.toBeNull()
  })

  it('parses the supported subset and shows the rest as written', () => {
    const blocks = text([
      '# Title',
      '## Section',
      '### Small',
      '#### Not a heading',
      'A paragraph with **bold**, `code` and a [link](https://example.com/x).',
      '- one',
      '* two **strong**',
      '> quoted',
      '> again',
      '',
      'plain *stars* stay',
    ].join('\n'))
    expect(blocks.map((block) => block.kind)).toEqual(['heading', 'heading', 'heading', 'paragraph', 'list', 'quote', 'paragraph'])
    const headings = blocks.filter((block) => block.kind === 'heading')
    expect(headings.map((block) => block.kind === 'heading' && block.level)).toEqual([1, 2, 3])
    const paragraph = blocks[3]
    expect(paragraph.kind === 'paragraph' && paragraph.children.map((node) => node.kind)).toEqual(['text', 'strong', 'text', 'code', 'text', 'link', 'text'])
    const list = blocks[4]
    expect(list.kind === 'list' && list.items).toHaveLength(2)
    const quote = blocks[5]
    expect(quote.kind === 'quote' && quote.lines).toHaveLength(2)
    // A fourth-level heading stays a paragraph, stars stay stars.
    expect(JSON.stringify(blocks[3])).toContain('#### Not a heading')
    expect(JSON.stringify(blocks[6])).toContain('plain *stars* stay')
  })

  it('nests emphasis inside bold and leaves unpaired markers alone', () => {
    expect(parseInline('**bold `code` **')).toEqual([
      { kind: 'strong', children: [{ kind: 'text', text: 'bold ' }, { kind: 'code', text: 'code' }, { kind: 'text', text: ' ' }] },
    ])
    expect(parseInline('an **open marker')).toEqual([{ kind: 'text', text: 'an **open marker' }])
    expect(parseInline('a `lone tick')).toEqual([{ kind: 'text', text: 'a `lone tick' }])
  })

  it('caps long notes and keeps what came before the cap', () => {
    const long = `${'x'.repeat(NOTES_LIMIT - 1)}**bold**
- item`
    const capped = truncateNotes(long)
    expect(capped.truncated).toBe(true)
    expect(capped.text).toHaveLength(NOTES_LIMIT)
    expect(capped.text.startsWith('x'.repeat(100))).toBe(true)
    // The saved text still belongs to a block the renderer can parse.
    expect(parseNotes(capped.text).length).toBeGreaterThan(0)
    expect(truncateNotes('short')).toEqual({ text: 'short', truncated: false })
  })
})
