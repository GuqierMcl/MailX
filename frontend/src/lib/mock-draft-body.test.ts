import { describe, expect, it } from 'vitest'
import { MAIL_COLORS, MAIL_HIGHLIGHTS } from '@/features/composer/mail-body-format'
import { mockDraftBody } from './mock-draft-body'

describe('mock draft rich formatting', () => {
  it('retains headings, per-paragraph alignment, and nested palette marks without CSS', () => {
    const html = '<h1 data-align="center">Title</h1><h2 data-align="right">Subtitle</h2><h3>Third</h3><p data-align="center"><span data-color="red"><mark data-highlight="yellow"><strong>Colored</strong></mark></span></p>'
    const body = mockDraftBody(html)
    expect(body.bodyHtml).toBe(html)
    expect(body.bodyText).toBe('Title\nSubtitle\nThird\nColored')
    expect(body.bodyHtml).not.toContain('style=')
  })

  it('supports every fixed palette name and does not add formatting to plain text', () => {
    for (const { name } of MAIL_COLORS) {
      const html = `<p><span data-color="${name}">Text</span></p>`
      expect(mockDraftBody(html)).toEqual({ bodyHtml: html, bodyText: 'Text' })
    }
    for (const { name } of MAIL_HIGHLIGHTS) {
      const html = `<p><mark data-highlight="${name}">Text</mark></p>`
      expect(mockDraftBody(html)).toEqual({ bodyHtml: html, bodyText: 'Text' })
    }
  })

  it('unwraps invalid color and highlight marks and strips mis-scoped or arbitrary attributes', () => {
    const body = mockDraftBody('<h1 data-align="left" style="text-align:center" onclick="bad()">A</h1><h2 data-align="justify" data-color="red">B</h2><p data-align="RIGHT"><span data-color="#dc2626">C</span><mark data-highlight="orange">D</mark><span class="red">E</span><mark>F</mark></p><span data-color="blue" data-align="right" data-highlight="yellow" style="color:red">G</span><mark data-highlight="gray" data-color="blue" id="x">H</mark>')
    expect(body.bodyHtml).toBe('<h1>A</h1><h2>B</h2><p>CDEF</p><span data-color="blue">G</span><mark data-highlight="gray">H</mark>')
    expect(body.bodyText).toBe('A\nB\nCDEF\nGH')
  })

  it('keeps the strict tag and URL boundary while permitting safe links and ordered starts', () => {
    const body = mockDraftBody('<script>secret</script><svg><text>foreign</text></svg><p><img src="https://tracker.invalid"><a href="javascript:bad()">bad</a><a href="https://">broken</a><a href=" https://example.com ">safe</a><a href="mailto:">empty</a></p><ol start="-2" data-align="center"><li><p data-align="right">Item</p></li></ol>')
    expect(body.bodyHtml).toBe('<p><a>bad</a><a>broken</a><a href="https://example.com">safe</a><a>empty</a></p><ol start="-2"><li><p data-align="right">Item</p></li></ol>')
    expect(body.bodyText).toContain('safe <https://example.com>')
    expect(body.bodyText).toContain('-2. Item')
    expect(body.bodyHtml).not.toMatch(/secret|foreign|tracker|javascript|style=/)
  })
})
