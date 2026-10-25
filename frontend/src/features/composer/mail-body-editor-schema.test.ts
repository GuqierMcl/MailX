import { Editor } from '@tiptap/core'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { mailEditorOptions } from './mail-body-editor-schema'
import {
  MAIL_COLORS, MAIL_HIGHLIGHTS,
  type MailColor, type MailHighlight, type MailTextAlign,
} from './mail-body-format'

const editors: Editor[] = []
beforeAll(() => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})
afterEach(() => { editors.splice(0).forEach((editor) => editor.destroy()) })

function engine(content = '<p></p>') {
  const editor = new Editor({ ...mailEditorOptions(), content })
  editors.push(editor)
  return editor
}

function key(editor: Editor, value: string, modifiers: KeyboardEventInit = {}) {
  const event = new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true, ...modifiers })
  editor.view.dom.dispatchEvent(event)
  return event
}

function typeText(editor: Editor, text: string) {
  for (const character of text) {
    const { from, to } = editor.state.selection
    const handled = editor.view.someProp('handleTextInput', (handler) => handler(editor.view, from, to, character, () => editor.state.tr.insertText(character, from, to)))
    if (!handled) editor.view.dispatch(editor.state.tr.insertText(character, from, to))
  }
}

describe('heading, alignment and palette mail schema', () => {
  it('supports three heading levels through shortcuts and Markdown input rules', () => {
    const editor = engine('<p>Title</p>')
    for (const level of [1, 2, 3] as const) {
      expect(key(editor, String(level), { ctrlKey: true, altKey: true }).defaultPrevented).toBe(true)
      expect(editor.getHTML()).toBe(`<h${level}>Title</h${level}>`)
    }
    expect(editor.commands.setHeading({ level: 4 })).toBe(false)
    expect(editor.commands.setParagraph()).toBe(true)
    expect(editor.getHTML()).toBe('<p>Title</p>')
    for (const level of [1, 2, 3]) {
      editor.commands.setContent('<p></p>')
      editor.commands.setTextSelection(1)
      typeText(editor, `${'#'.repeat(level)} 标题`)
      expect(editor.getHTML()).toBe(`<h${level}>标题</h${level}>`)
    }
  })

  it('only imports h1–h3 as heading nodes and excludes unsupported heading levels', () => {
    const editor = engine('<h1>One</h1><h2>Two</h2><h3>Three</h3><h4>Four</h4><h5>Five</h5><h6>Six</h6>')
    expect(editor.getHTML()).toBe('<h1>One</h1><h2>Two</h2><h3>Three</h3><p>Four</p><p>Five</p><p>Six</p>')
  })

  it('aligns all selected paragraphs and headings including nested list/quote content, with left represented by no attribute', () => {
    const editor = engine('<h2>Title</h2><p>Body</p><ul><li><p>Item</p></li></ul><blockquote><p>Quote</p></blockquote>')
    editor.commands.selectAll()
    expect(editor.commands.setMailTextAlign('center')).toBe(true)
    expect(editor.getHTML()).toBe('<h2 data-align="center">Title</h2><p data-align="center">Body</p><ul><li><p data-align="center">Item</p></li></ul><blockquote><p data-align="center">Quote</p></blockquote>')
    expect(editor.commands.setMailTextAlign('right')).toBe(true)
    expect(editor.getHTML()).not.toContain('center')
    expect(editor.getHTML().match(/data-align="right"/g)).toHaveLength(4)
    expect(editor.commands.setMailTextAlign('justify' as MailTextAlign)).toBe(false)
    expect(editor.commands.setMailTextAlign('left')).toBe(true)
    expect(editor.getHTML()).not.toContain('data-align')
    editor.commands.setTextSelection(2)
    editor.commands.setMailTextAlign('center')
    expect(editor.commands.unsetMailTextAlign()).toBe(true)
    expect(editor.getHTML()).not.toContain('data-align')
  })

  it('keeps alignment when switching a single paragraph to a heading and never binds alignment shortcuts', () => {
    const editor = engine('<p data-align="right">Text</p>')
    editor.commands.setTextSelection(2)
    editor.commands.setHeading({ level: 2 })
    expect(editor.getHTML()).toBe('<h2 data-align="right">Text</h2>')
    for (const character of ['L', 'E', 'R']) {
      expect(key(editor, character, { ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(false)
    }
    expect(editor.getHTML()).toBe('<h2 data-align="right">Text</h2>')
  })

  it('preserves pending typing marks when alignment changes and respects an explicit clear-format chain', () => {
    const editor = engine()
    editor.chain().toggleBold().setMailColor('red').setMailHighlight('yellow').run()
    editor.commands.setMailTextAlign('center')
    editor.commands.insertContent('Text')
    expect(editor.getHTML()).toBe('<p data-align="center"><strong><span data-color="red"><mark data-highlight="yellow">Text</mark></span></strong></p>')
    editor.chain().unsetAllMarks().command(({ tr }) => { tr.setStoredMarks([]); return true }).unsetMailTextAlign().run()
    editor.commands.insertContent(' plain')
    expect(editor.getHTML()).toBe('<p><strong><span data-color="red"><mark data-highlight="yellow">Text</mark></span></strong> plain</p>')
  })

  it('layers palette color and highlight with existing marks and removes each independently', () => {
    const editor = engine('<p><strong><a href="https://example.com">Text</a></strong></p>')
    editor.commands.selectAll()
    expect(editor.commands.setMailColor('red')).toBe(true)
    expect(editor.commands.setMailHighlight('yellow')).toBe(true)
    expect(editor.getAttributes('mailColor')).toEqual({ color: 'red' })
    expect(editor.getAttributes('mailHighlight')).toEqual({ highlight: 'yellow' })
    const html = editor.getHTML()
    expect(html).toContain('<span data-color="red">')
    expect(html).toContain('<mark data-highlight="yellow">')
    expect(html).toContain('<strong>')
    expect(html).toContain('<a href="https://example.com">')
    expect(html).not.toContain('style=')
    expect(editor.commands.unsetMailColor()).toBe(true)
    expect(editor.getHTML()).not.toContain('<span')
    expect(editor.getHTML()).toContain('data-highlight="yellow"')
    expect(editor.commands.unsetMailHighlight()).toBe(true)
    expect(editor.getHTML()).not.toContain('<mark')
    expect(editor.getHTML()).toContain('<strong>')
  })

  it('round-trips every fixed color and highlight using named data attributes only', () => {
    const editor = engine('<p>Text</p>')
    editor.commands.selectAll()
    for (const { name } of MAIL_COLORS) {
      expect(editor.commands.setMailColor(name)).toBe(true)
      const html = `<p><span data-color="${name}">Text</span></p>`
      expect(editor.getHTML()).toBe(html)
      expect(engine(html).getHTML()).toBe(html)
    }
    editor.commands.unsetMailColor()
    for (const { name } of MAIL_HIGHLIGHTS) {
      expect(editor.commands.setMailHighlight(name)).toBe(true)
      const html = `<p><mark data-highlight="${name}">Text</mark></p>`
      expect(editor.getHTML()).toBe(html)
      expect(engine(html).getHTML()).toBe(html)
    }
  })

  it('rejects arbitrary colors and unsupported highlight names at both command and parse boundaries', () => {
    const editor = engine('<p><span data-color="#ff0000">CSS</span><span data-color="RED">Case</span><mark data-highlight="orange">Orange</mark><span style="color:red">Style</span><mark>Default</mark></p><p data-align="justify">Align</p>')
    expect(editor.getHTML()).toBe('<p>CSSCaseOrangeStyleDefault</p><p>Align</p>')
    editor.commands.selectAll()
    expect(editor.commands.setMailColor('#ff0000' as MailColor)).toBe(false)
    expect(editor.commands.setMailHighlight('orange' as MailHighlight)).toBe(false)
    expect(editor.commands.setMailTextAlign('justify' as MailTextAlign)).toBe(false)
    expect(editor.getHTML()).not.toMatch(/data-|style=/)
  })

  it('clears color, highlight and alignment while preserving heading block type', () => {
    const editor = engine('<h2 data-align="center"><strong><span data-color="blue"><mark data-highlight="yellow">Title</mark></span></strong></h2>')
    editor.commands.selectAll()
    editor.chain().unsetAllMarks().unsetMailTextAlign().run()
    expect(editor.getHTML()).toBe('<h2>Title</h2>')
  })

  it('strips external CSS and forged palette attributes on paste while retaining valid supported alignment', () => {
    const editor = engine()
    editor.view.pasteHTML('<h2 data-align="center" style="color:red;text-align:right" class="bad">Title</h2><p data-align="right"><span data-color="red" style="color:blue">Red</span><mark data-highlight="yellow">Yellow</mark></p><p data-align="justify" align="right">No alignment</p><div data-align="center">Container</div><blockquote data-align="right"><p>Quote</p></blockquote><p data-color="purple" data-highlight="blue" data-align="left">No attributes</p>', new Event('paste') as ClipboardEvent)
    expect(editor.getHTML()).toBe('<h2 data-align="center">Title</h2><p data-align="right">RedYellow</p><p>No alignment</p><p>Container</p><blockquote><p>Quote</p></blockquote><p>No attributes</p>')
    expect(editor.getHTML()).not.toMatch(/style=|class=|data-color=|data-highlight=|<span|<mark/)
  })
})
