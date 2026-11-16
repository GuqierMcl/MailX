import { describe, expect, it } from 'vitest'
import { parseBlock, parseInline, parseMarkdown, safeHref, splitBlocks } from './markdown'

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
    // A half-written link never becomes a link labelled with the text; the address alone is a plain address.
    expect(parseInline('[docs](', true)).toEqual([{ type: 'text', text: '[docs](' }])
    expect(parseInline('[docs](https://exa', true).some((node) => node.type === 'link' && node.children[0]?.type === 'text' && node.children[0].text === 'docs')).toBe(false)
  })

  it('does not treat spaced stars or snake_case as emphasis', () => {
    expect(parseInline('2 * 3 * 4')).toEqual([{ type: 'text', text: '2 * 3 * 4' }])
    expect(parseInline('snake_case_name')).toEqual([{ type: 'text', text: 'snake_case_name' }])
  })

  it('takes a backslash escape literally', () => {
    expect(parseInline('\\*not\\*')).toEqual([{ type: 'text', text: '*not*' }])
  })
})

describe('blocks', () => {
  it('splits paragraphs at blank lines and keeps a code fence whole', () => {
    expect(splitBlocks('a\n\nb\nc\n\n```\nx\n\ny\n```\n\nd')).toEqual(['a', 'b\nc', '```\nx\n\ny\n```', 'd'])
  })

  it('keeps an unfinished fence as one piece', () => {
    expect(splitBlocks('intro\n```\nlet a')).toEqual(['intro', '```\nlet a'])
    expect(parseMarkdown('```\nlet a\n\nlet b')).toEqual([{ type: 'code', text: 'let a\n\nlet b' }])
  })

  it('reads bulleted and numbered lists, with their start and nesting', () => {
    expect(parseBlock('- one\n- two\n  - inner')).toEqual([{
      type: 'list', ordered: false, start: 1,
      items: [
        [{ type: 'paragraph', children: [{ type: 'text', text: 'one' }] }],
        [{ type: 'paragraph', children: [{ type: 'text', text: 'two' }] }, { type: 'list', ordered: false, start: 1, items: [[{ type: 'paragraph', children: [{ type: 'text', text: 'inner' }] }]] }],
      ],
    }])
    expect(parseMarkdown('1. a\n\n2. b')).toMatchObject([{ ordered: true, start: 1 }, { ordered: true, start: 2 }])
  })

  it('reads a blockquote and a heading', () => {
    expect(parseMarkdown('> quoted\n> more')).toEqual([{ type: 'quote', children: [{ type: 'paragraph', children: [{ type: 'text', text: 'quoted' }, { type: 'br' }, { type: 'text', text: 'more' }] }] }])
    expect(parseMarkdown('## Title')).toEqual([{ type: 'heading', children: [{ type: 'text', text: 'Title' }] }])
  })

  it('treats a streamed half list item as a list, and only the last block as open', () => {
    expect(parseMarkdown('intro\n\n- a\n- **b', true)).toMatchObject([
      { type: 'paragraph' },
      { type: 'list', items: [[{ type: 'paragraph' }], [{ type: 'paragraph', children: [{ type: 'strong' }] }]] },
    ])
  })

  it('does not throw on partial input at any length', () => {
    const full = 'Hi **there**,\n\n- [a](https://x.y/z) and `c`\n- 2. *e*\n\n> q\n\n```\ncode\n```\n'
    for (let at = 0; at <= full.length; at += 1) expect(() => parseMarkdown(full.slice(0, at), true)).not.toThrow()
  })
})
