import { Editor } from '@tiptap/core'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { mailEditorOptions } from './mail-body-editor-schema'

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

function typeText(editor: Editor, text: string) {
  for (const character of text) {
    const { from, to } = editor.state.selection
    const handled = editor.view.someProp('handleTextInput', (handler) => handler(editor.view, from, to, character, () => editor.state.tr.insertText(character, from, to)))
    if (!handled) editor.view.dispatch(editor.state.tr.insertText(character, from, to))
  }
}

function paste(editor: Editor, text: string, html = '') {
  // view.pasteText explicitly requests plain paste. A real clipboard event is
  // required here to exercise the normal Markdown/rich-text paste path.
  const event = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'clipboardData', { value: { getData: (type: string) => type === 'text/html' ? html : type === 'text/plain' ? text : '' } })
  editor.view.dom.dispatchEvent(event)
}

describe('official Tiptap Markdown integration', () => {
  it('uses the official Markdown commands and serializer while keeping drafts in HTML', () => {
    const editor = engine('<p>**已有草稿原文**</p>')
    expect(editor.getHTML()).toBe('<p>**已有草稿原文**</p>')
    editor.commands.setContent('## 标题\n\n**粗体**', { contentType: 'markdown' })
    expect(editor.getHTML()).toBe('<h2>标题</h2><p><strong>粗体</strong></p>')
    expect(editor.getMarkdown()).toBe('## 标题\n\n**粗体**')
    editor.commands.setContent('<p><em>保存重载</em></p>')
    expect(editor.getHTML()).toBe('<p><em>保存重载</em></p>')
  })

  it.each([
    ['**粗体**', '<strong>粗体</strong>'],
    ['__粗体__', '<strong>粗体</strong>'],
    ['*斜体*', '<em>斜体</em>'],
    ['_斜体_', '<em>斜体</em>'],
    ['~~删除~~', '<s>删除</s>'],
    ['[文档](https://example.com/path)', '<a href="https://example.com/path">文档</a>'],
    ['[联系](mailto:hello@example.com)', '<a href="mailto:hello@example.com">联系</a>'],
  ])('converts typed %s and ends the formatting after the delimiter', (source, html) => {
    const editor = engine()
    typeText(editor, `${source} `)
    expect(editor.getHTML()).toBe(`<p>${html} </p>`)
  })

  it.each(['[坏链接](javascript:alert(1))', '[坏链接](data:text/html,test)', '![图片](https://example.com/a.png)', '\\[文档](https://example.com)', '`[文档](https://example.com)`', '#### 标题'])('keeps unsupported or escaped typed syntax literal: %s', (source) => {
    const editor = engine()
    typeText(editor, source)
    expect(editor.getText()).toBe(source)
    expect(editor.getHTML()).not.toMatch(/<a |<img|<h[1-6]>/)
  })

  it('immediately restores input syntax with Backspace or undo', () => {
    const editor = engine()
    typeText(editor, '**粗体**')
    expect(editor.getHTML()).toBe('<p><strong>粗体</strong></p>')
    editor.view.dom.dispatchEvent(new KeyboardEvent('keydown', { key: 'Backspace', bubbles: true, cancelable: true }))
    expect(editor.getHTML()).toBe('<p>**粗体**</p>')
    editor.commands.setContent('<p></p>')
    typeText(editor, '# ')
    expect(editor.getHTML()).toBe('<h1></h1>')
    expect(editor.commands.undo()).toBe(true)
    expect(editor.getHTML()).toBe('<p># </p>')
  })

  it('pastes whole Markdown at the selection and undoes it in one step', () => {
    const editor = engine('<p>原文</p>')
    editor.commands.selectAll()
    paste(editor, '# 标题\n\n**粗体** 和 _斜体_\n\n- 第一项\n  - 子项\n\n> 引用\n\n[文档](https://example.com)')
    expect(editor.getHTML()).toContain('<h1>标题</h1>')
    expect(editor.getHTML()).toContain('<strong>粗体</strong> 和 <em>斜体</em>')
    expect(editor.getHTML()).toContain('<ul><li><p>第一项</p><ul><li><p>子项</p></li></ul></li></ul>')
    expect(editor.getHTML()).toContain('<blockquote><p>引用</p></blockquote>')
    expect(editor.getHTML()).toContain('<a href="https://example.com">文档</a>')
    expect(editor.commands.undo()).toBe(true)
    expect(editor.getHTML()).toBe('<p>原文</p>')
  })

  it('leaves escaped emphasis literal without a second Markdown pass', () => {
    const editor = engine()
    paste(editor, '**格式**\n\n\\*保留\\*')
    expect(editor.getHTML()).toContain('<strong>格式</strong>')
    expect(editor.getHTML()).toContain('<p>*保留*</p>')
  })

  it('applies the mail URI whitelist to the official Markdown parser', () => {
    const editor = engine()
    editor.commands.setContent('[正常](https://example.com) [危险](javascript:alert(1)) [文件](file:///tmp/secret)', { contentType: 'markdown' })
    expect(editor.getHTML()).toBe('<p><a href="https://example.com">正常</a> 危险 文件</p>')
    const links: string[] = []
    editor.state.doc.descendants((node) => { node.marks.forEach((mark) => { if (mark.type.name === 'link') links.push(mark.attrs.href) }) })
    expect(links).toEqual(['https://example.com'])
  })

  it('applies the existing HTML paste boundary to raw HTML inside Markdown', () => {
    const editor = engine()
    paste(editor, '# 标题\n\n<span data-color="red" style="color:red">颜色</span> <mark data-highlight="yellow">高亮</mark>\n\n<img src="https://tracker.invalid/a.png" onerror="alert(1)">\n\n<script>alert(2)</script>')
    expect(editor.getHTML()).toContain('<h1>标题</h1>')
    expect(editor.getText()).toContain('颜色')
    expect(editor.getText()).toContain('高亮')
    expect(editor.getHTML()).not.toMatch(/<img|<script|<span|<mark|data-color=|data-highlight=|style=/)
  })

  it.each(['```\ncode\n```', '`code`'])('falls back to text when the official parser produces no supported content: %s', (text) => {
    const editor = engine('<p>选中内容</p>')
    editor.commands.selectAll()
    paste(editor, text)
    expect(editor.getText()).toBe(text.replaceAll('\n', '\n\n'))
  })

  it('prioritizes clipboard HTML and applies the existing sanitizer', () => {
    const editor = engine()
    paste(editor, '# Markdown', '<p><em>HTML</em><img src="https://example.com/image.png"><script>secret</script></p>')
    expect(editor.getHTML()).toBe('<p><em>HTML</em></p>')
  })

  it('supports Shift paste without converting Markdown or automatically linking its source', () => {
    const editor = engine()
    editor.view.dom.dispatchEvent(new KeyboardEvent('keydown', { key: 'v', ctrlKey: true, shiftKey: true, bubbles: true }))
    paste(editor, '# 标题\n**原文**\nhttps://example.com', '<h1>标题</h1>')
    editor.view.dom.dispatchEvent(new KeyboardEvent('keyup', { key: 'Shift', bubbles: true }))
    expect(editor.getHTML()).toBe('<p># 标题</p><p>**原文**</p><p>https://example.com</p>')
  })

  it('retains Tiptap URL-on-selection behavior', () => {
    const editor = engine('<p>文档</p>')
    editor.commands.selectAll()
    paste(editor, 'https://example.com')
    expect(editor.getHTML()).toBe('<p><a href="https://example.com">文档</a></p>')
  })
})
