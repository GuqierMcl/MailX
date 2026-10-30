import { i18n } from '@/i18n'
import { createRef, lazy, StrictMode, Suspense } from 'react'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Editor } from '@tiptap/core'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import MailBodyEditor, { type MailBodyEditorHandle } from './mail-body-editor'
import { allowedMailLink, mailEditorOptions } from './mail-body-editor-schema'

const editors: Editor[] = []
beforeAll(() => {
  // jsdom has no layout. ProseMirror asks ranges for caret rectangles when
  // restoring focus or scrolling a selection after a formatting command.
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  document.elementFromPoint = () => null
})
afterEach(() => {
  editors.splice(0).forEach((editor) => editor.destroy())
  void i18n.changeLanguage('zh-CN')
})

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

function key(editor: Editor, value: string, modifiers: KeyboardEventInit = {}) {
  const event = new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true, ...modifiers })
  editor.view.dom.dispatchEvent(event)
  return event
}

describe('mail editor schema and clipboard', () => {
  it('keeps only supported formatting and strips active content, attributes, styles and images on paste', () => {
    const editor = engine()
    editor.view.pasteHTML('<h1>Heading</h1><p style="color:red;font-weight:bold" onclick="alert(1)">Plain <b class="bad">bold</b><i>italic</i><u>underlined</u><del>deleted</del><img src="https://tracker.invalid" onerror="alert(1)"></p><script>secret()</script><style>body{display:none}</style><svg><text>SVG</text></svg><iframe>iframe</iframe><ol start="3" class="bad"><li>third</li></ol><p><a href="javascript:alert(1)">bad</a> <a href="https://example.com" target="_blank" rel="opener" title="bad">safe</a></p>', new Event('paste') as ClipboardEvent)
    const html = editor.getHTML()
    expect(html).toContain('<h1>Heading</h1><p>Plain <strong>bold</strong><em>italic</em><u>underlined</u><s>deleted</s></p>')
    expect(html).toContain('<ol start="3"><li><p>third</p></li></ol>')
    expect(html).toContain('<a href="https://example.com">safe</a>')
    expect(html).not.toMatch(/style=|onclick=|class=|target=|rel=|title=|javascript:|img|secret|SVG|iframe/)
    expect(Object.keys(editor.schema.nodes).sort()).toEqual(['blockquote', 'bulletList', 'doc', 'hardBreak', 'heading', 'inlineImage', 'listItem', 'orderedList', 'paragraph', 'text'].sort())
  })

  it('explicitly pastes as plain text with separate paragraphs and no Markdown formatting', () => {
    const editor = engine()
    editor.view.pasteText('first\r\n\r\n**bold**\n_italic_\n- item\n> quote', new Event('paste') as ClipboardEvent)
    expect(editor.getHTML()).toBe('<p>first</p><p></p><p>**bold**</p><p>_italic_</p><p>- item</p><p>&gt; quote</p>')
  })

  it('preserves whitespace in pasted plain text', () => {
    const editor = engine()
    editor.view.pasteText('  A  B\tC\n  D', new Event('paste') as ClipboardEvent)
    expect(editor.getHTML()).toBe('<p>  A  B\tC</p><p>  D</p>')
  })

  it('automatically links typed and pasted URLs while rejecting other protocols', () => {
    const editor = engine()
    typeText(editor, 'https://example.com ')
    expect(editor.getHTML()).toContain('<a href="https://example.com">https://example.com</a>')
    editor.commands.setContent('<p></p>')
    const event = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(event, 'clipboardData', { value: { getData: (type: string) => type === 'text/plain' ? 'see https://example.com/path and mailto:hello@example.com' : '' } })
    editor.view.dom.dispatchEvent(event)
    expect(editor.getHTML()).toContain('<a href="https://example.com/path">https://example.com/path</a>')
    expect(editor.getHTML()).toContain('<a href="mailto:hello@example.com">hello@example.com</a>')
    for (const href of ['javascript:alert(1)', 'data:text/html,hi', 'ftp://example.com', '//example.com', 'file:///tmp/test', 'https:\n//example.com', 'https://']) {
      expect(allowedMailLink(href)).toBe(false)
      expect(editor.commands.setLink({ href })).toBe(false)
    }
  })

  it('supports Markdown emphasis, list and quote input rules, and nesting', () => {
    const editor = engine()
    typeText(editor, '**normal** ')
    expect(editor.getHTML()).toBe('<p><strong>normal</strong> </p>')
    for (const [text, tag] of [['- ', 'ul'], ['* ', 'ul'], ['1. ', 'ol'], ['> ', 'blockquote']]) {
      editor.commands.setContent('<p></p>')
      editor.commands.setTextSelection(1)
      typeText(editor, text)
      expect(editor.getHTML()).toContain(`<${tag}>`)
    }
    typeText(editor, '> ')
    expect(editor.getHTML()).toContain('<blockquote><blockquote>')
    editor.commands.setContent('<ul><li><p>one</p></li><li><p>two</p></li></ul>')
    editor.commands.setTextSelection(10)
    expect(editor.commands.sinkListItem('listItem')).toBe(true)
    expect(editor.getHTML()).toBe('<ul><li><p>one</p><ul><li><p>two</p></li></ul></li></ul>')
  })

  it('leaves Mod+Enter for sending, keeps Shift+Enter and treats Enter as a new paragraph', () => {
    const editor = engine('<p>body</p>')
    editor.commands.setTextSelection(5)
    expect(key(editor, 'Enter', { ctrlKey: true }).defaultPrevented).toBe(false)
    expect(key(editor, 'Enter', { metaKey: true }).defaultPrevented).toBe(false)
    expect(editor.getHTML()).toBe('<p>body</p>')
    expect(key(editor, 'Enter', { shiftKey: true }).defaultPrevented).toBe(true)
    expect(editor.getHTML()).toBe('<p>body<br></p>')
    key(editor, 'Enter')
    expect(editor.getHTML()).toBe('<p>body<br></p><p></p>')
  })

  it('does not inject a style element into the strict-CSP page', () => {
    engine()
    expect(document.querySelector('style[data-tiptap-style]')).toBeNull()
  })
})

function renderEditor(value = '<p>Hello</p>', readOnly = false) {
  const onChange = vi.fn()
  const editorRef = createRef<MailBodyEditorHandle>()
  const component = (html: string, locked: boolean) => <TooltipProvider><MailBodyEditor value={html} onChange={onChange} readOnly={locked} editorRef={editorRef} /></TooltipProvider>
  const view = render(component(value, readOnly))
  return { onChange, editorRef, unmount: view.unmount, rerender: (html: string, locked = readOnly) => view.rerender(component(html, locked)) }
}

describe('MailBodyEditor', () => {
  it('preserves whitespace on initial load, equivalent save acknowledgment, reopening and external reload', async () => {
    const text = '  A  B\tC'
    const initial = renderEditor(`<p>${text}</p>`)
    const body = await screen.findByRole('textbox', { name: '正文' })
    expect(body.textContent).toBe(text)
    expect(initial.onChange).not.toHaveBeenCalled()
    act(() => initial.editorRef.current?.setSelection({ from: 1, to: text.length + 1 }))
    await userEvent.click(screen.getByRole('button', { name: '粗体' }))
    const saved = `<p><strong>${text}</strong></p>`
    expect(initial.onChange).toHaveBeenLastCalledWith(saved)
    act(() => initial.editorRef.current?.setSelection({ from: 3, to: 3 }))
    initial.rerender(`<p><b>${text}</b></p>`)
    expect(initial.editorRef.current?.getSelection()).toEqual({ from: 3, to: 3 })
    expect(screen.getByRole('button', { name: '撤销' })).toBeEnabled()
    expect(body.textContent).toBe(text)
    initial.unmount()
    const reopened = renderEditor(saved)
    expect((await screen.findByRole('textbox', { name: '正文' })).textContent).toBe(text)
    expect(reopened.onChange).not.toHaveBeenCalled()
    reopened.rerender('<p>  Updated  \ttext</p>')
    expect(screen.getByRole('textbox', { name: '正文' }).textContent).toBe('  Updated  \ttext')
  })

  it('mounts and focuses safely when lazily loaded under StrictMode', async () => {
    const LazyEditor = lazy(async () => ({ default: MailBodyEditor }))
    const onChange = vi.fn()
    const { rerender } = render(<StrictMode><TooltipProvider><Suspense fallback="Loading"><LazyEditor value="<p></p><blockquote><p>Original</p></blockquote>" onChange={onChange} /></Suspense></TooltipProvider></StrictMode>)
    const body = await screen.findByRole('textbox', { name: '正文' })
    await waitFor(() => expect(body).toHaveFocus())
    expect(window.getSelection()?.anchorOffset).toBe(0)
    expect(onChange).not.toHaveBeenCalled()
    rerender(<StrictMode><TooltipProvider><Suspense fallback="Loading"><LazyEditor value="<p>Locked</p>" onChange={onChange} readOnly /></Suspense></TooltipProvider></StrictMode>)
    expect(screen.getByRole('textbox', { name: '正文' })).toHaveAttribute('contenteditable', 'false')
  })

  it('keeps selection on toolbar formatting, updates pressed state, and supports undo/redo and clear formatting', async () => {
    const { onChange, editorRef } = renderEditor()
    await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    await userEvent.click(screen.getByRole('button', { name: '粗体' }))
    expect(onChange).toHaveBeenLastCalledWith('<p><strong>Hello</strong></p>')
    expect(screen.getByRole('button', { name: '粗体' })).toHaveAttribute('aria-pressed', 'true')
    expect(editorRef.current?.getSelection()).toEqual({ from: 1, to: 6 })
    await userEvent.click(screen.getByRole('button', { name: '撤销' }))
    expect(onChange).toHaveBeenLastCalledWith('<p>Hello</p>')
    await userEvent.click(screen.getByRole('button', { name: '重做' }))
    expect(onChange).toHaveBeenLastCalledWith('<p><strong>Hello</strong></p>')
    await userEvent.click(screen.getByRole('button', { name: '清除格式' }))
    expect(onChange).toHaveBeenLastCalledWith('<p>Hello</p>')
  })

  it('does not emit initial content and preserves the caret for an equivalent saved HTML value', async () => {
    const { onChange, editorRef, rerender } = renderEditor('<p><b>Hello</b></p>')
    await screen.findByRole('textbox', { name: '正文' })
    expect(onChange).not.toHaveBeenCalled()
    act(() => editorRef.current?.setSelection({ from: 3, to: 3 }))
    rerender('<p><strong>Hello</strong></p>')
    expect(editorRef.current?.getSelection()).toEqual({ from: 3, to: 3 })
    expect(onChange).not.toHaveBeenCalled()
    rerender('<p>Reloaded</p>')
    expect(screen.getByRole('textbox', { name: '正文' })).toHaveTextContent('Reloaded')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('edits links with Mod+K, validates the protocol and removes links without navigation', async () => {
    const { onChange, editorRef } = renderEditor()
    const body = await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    fireEvent.keyDown(body, { key: 'k', ctrlKey: true })
    const address = screen.getByRole('textbox', { name: '链接地址' })
    expect(address).toHaveFocus()
    await userEvent.type(address, 'javascript:alert(1)')
    await userEvent.click(screen.getByRole('button', { name: '应用' }))
    expect(screen.getByRole('alert')).toHaveTextContent('http、https 或 mailto')
    expect(onChange).not.toHaveBeenCalled()
    await userEvent.clear(address)
    await userEvent.type(address, 'https://example.com')
    await userEvent.click(screen.getByRole('button', { name: '应用' }))
    expect(onChange).toHaveBeenLastCalledWith('<p><a href="https://example.com">Hello</a></p>')
    const open = vi.spyOn(window, 'open')
    await userEvent.click(within(body).getByRole('link', { name: 'Hello' }))
    expect(open).not.toHaveBeenCalled()
    expect(screen.getByRole('textbox', { name: '链接地址' })).toHaveValue('https://example.com')
    await userEvent.click(screen.getByRole('button', { name: '移除链接' }))
    expect(onChange).toHaveBeenLastCalledWith('<p>Hello</p>')
  })

  it('keeps locked drafts read-only including toolbar, shortcuts, link clicks and an already-open popup', async () => {
    const value = '<p><a href="https://example.com">Hello</a></p>'
    const { onChange, rerender } = renderEditor(value)
    const body = await screen.findByRole('textbox', { name: '正文' })
    fireEvent.keyDown(body, { key: 'k', ctrlKey: true })
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    rerender(value, true)
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(body).toHaveAttribute('contenteditable', 'false')
    expect(body).toHaveAttribute('aria-readonly', 'true')
    expect(screen.getByRole('button', { name: '粗体' })).toBeDisabled()
    fireEvent.keyDown(body, { key: 'k', ctrlKey: true })
    const click = new MouseEvent('click', { bubbles: true, cancelable: true })
    within(body).getByRole('link').dispatchEvent(click)
    expect(click.defaultPrevented).toBe(true)
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('does not apply a link during IME composition or forward the send shortcut from its input', async () => {
    const { onChange, editorRef } = renderEditor()
    await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    await userEvent.click(screen.getByRole('button', { name: '链接' }))
    const address = screen.getByRole('textbox', { name: '链接地址' })
    await userEvent.type(address, 'https://example.com')
    const send = vi.fn()
    document.addEventListener('keydown', send)
    try {
      fireEvent.keyDown(address, { key: 'Enter', ctrlKey: true })
      fireEvent.keyDown(address, { key: 'Enter', metaKey: true })
      fireEvent.compositionStart(address)
      fireEvent.keyDown(address, { key: 'Enter', isComposing: true, keyCode: 229 })
      fireEvent.submit(address.closest('form')!)
      expect(onChange).not.toHaveBeenCalled()
      expect(send).not.toHaveBeenCalled()
      fireEvent.compositionEnd(address)
      await userEvent.click(screen.getByRole('button', { name: '应用' }))
      expect(onChange).toHaveBeenLastCalledWith('<p><a href="https://example.com">Hello</a></p>')
    } finally {
      document.removeEventListener('keydown', send)
    }
  })

  it('closes a stale link range when a conflict reload replaces the body', async () => {
    const { editorRef, rerender, onChange } = renderEditor()
    await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    await userEvent.click(screen.getByRole('button', { name: '链接' }))
    expect(screen.getByRole('dialog', { name: '编辑链接' })).toBeInTheDocument()
    rerender('<p>x</p>')
    expect(screen.queryByRole('dialog', { name: '编辑链接' })).not.toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: '正文' })).toHaveTextContent('x')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('changes toolbar and accessible text while preserving the body, selection and undo history', async () => {
    const { editorRef, onChange } = renderEditor()
    const body = await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    await userEvent.click(screen.getByRole('button', { name: '粗体' }))
    const html = body.innerHTML
    const selection = editorRef.current?.getSelection()
    onChange.mockClear()
    await act(async () => { await i18n.changeLanguage('en') })
    expect(screen.getByRole('textbox', { name: 'Message body' })).toBe(body)
    expect(body.innerHTML).toBe(html)
    expect(editorRef.current?.getSelection()).toEqual(selection)
    expect(screen.getByRole('button', { name: 'Undo' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Bold' })).toBeInTheDocument()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('keeps undo for equivalent save acknowledgments but drops local history after a conflict reload', async () => {
    const { editorRef, rerender, onChange } = renderEditor()
    const body = await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    await userEvent.click(screen.getByRole('button', { name: '粗体' }))
    rerender('<p><b>Hello</b></p>')
    expect(screen.getByRole('button', { name: '撤销' })).toBeEnabled()
    expect(editorRef.current?.getSelection()).toEqual({ from: 1, to: 6 })
    await userEvent.click(screen.getByRole('button', { name: '撤销' }))
    expect(onChange).toHaveBeenLastCalledWith('<p>Hello</p>')
    await userEvent.click(screen.getByRole('button', { name: '斜体' }))
    expect(screen.getByRole('button', { name: '撤销' })).toBeEnabled()
    onChange.mockClear()
    rerender('<p>Remote version</p>')
    expect(editorRef.current?.getSelection()).toEqual({ from: 1, to: 1 })
    expect(screen.getByRole('button', { name: '撤销' })).toBeDisabled()
    expect(screen.getByRole('button', { name: '重做' })).toBeDisabled()
    fireEvent.keyDown(body, { key: 'z', ctrlKey: true })
    expect(body).toHaveTextContent('Remote version')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('provides a keyboard-operable more menu and Escape restores the editor', async () => {
    const { onChange, editorRef } = renderEditor()
    const body = await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    await userEvent.click(screen.getByRole('button', { name: '更多格式' }))
    const menu = screen.getByRole('menu', { name: '更多正文格式' })
    act(() => within(menu).getByRole('menuitem', { name: '左对齐' }).focus())
    await userEvent.keyboard('{ArrowRight}{Enter}')
    expect(onChange).toHaveBeenLastCalledWith('<p data-align="center">Hello</p>')
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '更多格式' }))
    await userEvent.keyboard('{Escape}')
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    await waitFor(() => expect(body).toHaveFocus())
  })

  it('changes paragraph styles and alignment while clear formatting preserves the heading', async () => {
    const { onChange, editorRef } = renderEditor()
    const body = await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    await userEvent.click(screen.getByRole('button', { name: '段落样式：正文' }))
    expect(screen.getByRole('menuitemradio', { name: '正文' })).toHaveFocus()
    await userEvent.keyboard('{ArrowDown}{ArrowDown}{Enter}')
    expect(onChange).toHaveBeenLastCalledWith('<h2>Hello</h2>')
    expect(screen.getByRole('button', { name: '段落样式：标题 2' })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '居中对齐' }))
    expect(onChange).toHaveBeenLastCalledWith('<h2 data-align="center">Hello</h2>')
    expect(screen.getByRole('button', { name: '居中对齐' })).toHaveAttribute('aria-pressed', 'true')
    await userEvent.click(screen.getByRole('button', { name: '粗体' }))
    await userEvent.click(screen.getByRole('button', { name: '清除格式' }))
    expect(onChange).toHaveBeenLastCalledWith('<h2>Hello</h2>')
    expect(body).toHaveFocus()
    await userEvent.click(screen.getByRole('button', { name: '段落样式：标题 2' }))
    await userEvent.keyboard('{Home}{Enter}')
    expect(onChange).toHaveBeenLastCalledWith('<p>Hello</p>')
  })

  it('operates palettes by keyboard, combines colors and restores the exact editor selection', async () => {
    const { onChange, editorRef } = renderEditor()
    const body = await screen.findByRole('textbox', { name: '正文' })
    const range = { from: 2, to: 5 }
    act(() => editorRef.current?.setSelection(range))
    await userEvent.click(screen.getByRole('button', { name: '文字颜色' }))
    expect(screen.getByRole('menuitemradio', { name: '自动' })).toHaveFocus()
    await userEvent.keyboard('{ArrowRight}{ArrowRight}{Enter}')
    expect(onChange).toHaveBeenLastCalledWith('<p>H<span data-color="red">ell</span>o</p>')
    expect(body).toHaveFocus()
    expect(editorRef.current?.getSelection()).toEqual(range)
    await userEvent.click(screen.getByRole('button', { name: '文字颜色' }))
    expect(screen.getByRole('menuitemradio', { name: '红色' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('menuitemradio', { name: '红色' })).toHaveFocus()
    await userEvent.keyboard('{Escape}')
    expect(editorRef.current?.getSelection()).toEqual(range)
    await userEvent.click(screen.getByRole('button', { name: '高亮' }))
    await userEvent.keyboard('{ArrowDown}{Enter}')
    expect(body.querySelector('mark')).toHaveAttribute('data-highlight', 'yellow')
    expect(body.querySelector('span[data-color="red"]')).toHaveTextContent('ell')
    expect(body.querySelector('[style]')).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: '高亮' }))
    await userEvent.click(screen.getByRole('menuitemradio', { name: '无' }))
    expect(body.querySelector('mark')).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: '文字颜色' }))
    await userEvent.click(screen.getByRole('menuitemradio', { name: '自动' }))
    expect(onChange).toHaveBeenLastCalledWith('<p>Hello</p>')
  })

  it('discards stale palette ranges on reload and makes new controls read-only with the draft', async () => {
    const { editorRef, rerender, onChange } = renderEditor()
    await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 6 }))
    await userEvent.click(screen.getByRole('button', { name: '文字颜色' }))
    rerender('<h3 data-align="right"><span data-color="blue">Remote</span></h3>')
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '段落样式：标题 3' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '右对齐' })).toHaveAttribute('aria-pressed', 'true')
    await userEvent.click(screen.getByRole('button', { name: '高亮' }))
    rerender('<h3 data-align="right"><span data-color="blue">Remote</span></h3>', true)
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    for (const name of ['段落样式：标题 3', '文字颜色', '高亮', '左对齐', '居中对齐', '右对齐']) {
      expect(screen.getByRole('button', { name })).toBeDisabled()
    }
    expect(onChange).not.toHaveBeenCalled()
  })

  it('does not forward send shortcuts or composing Enter from a palette', async () => {
    const { onChange } = renderEditor()
    await screen.findByRole('textbox', { name: '正文' })
    await userEvent.click(screen.getByRole('button', { name: '文字颜色' }))
    const selected = screen.getByRole('menuitemradio', { name: '自动' })
    const send = vi.fn()
    document.addEventListener('keydown', send)
    try {
      fireEvent.keyDown(selected, { key: 'Enter', ctrlKey: true })
      fireEvent.keyDown(selected, { key: 'Enter', metaKey: true })
      fireEvent.keyDown(selected, { key: 'Enter', isComposing: true, keyCode: 229 })
      expect(send).not.toHaveBeenCalled()
      expect(onChange).not.toHaveBeenCalled()
      expect(screen.getByRole('menu', { name: '文字颜色' })).toBeInTheDocument()
    } finally {
      document.removeEventListener('keydown', send)
    }
  })


  it('keeps pending bold, color and highlight when formatting an empty heading before typing', async () => {
    const { editorRef } = renderEditor('<p></p>')
    const body = await screen.findByRole('textbox', { name: '正文' })
    act(() => editorRef.current?.setSelection({ from: 1, to: 1 }))
    await userEvent.click(screen.getByRole('button', { name: '粗体' }))
    await userEvent.click(screen.getByRole('button', { name: '文字颜色' }))
    await userEvent.click(screen.getByRole('menuitemradio', { name: '红色' }))
    expect(screen.getByRole('button', { name: '粗体' })).toHaveAttribute('aria-pressed', 'true')
    await userEvent.click(screen.getByRole('button', { name: '高亮' }))
    await userEvent.click(screen.getByRole('menuitemradio', { name: '黄色' }))
    expect(body).toHaveFocus()
    await userEvent.click(screen.getByRole('button', { name: '段落样式：正文' }))
    await userEvent.click(screen.getByRole('menuitemradio', { name: '标题 2' }))
    await userEvent.click(screen.getByRole('button', { name: '居中对齐' }))
    // Tiptap exposes its editor on the mounted DOM element. Dispatch the same
    // text-input transaction as the schema tests; jsdom has no native editing.
    act(() => typeText((body as HTMLElement & { editor: Editor }).editor, 'Text'))
    expect(body.querySelector('h2')).toHaveAttribute('data-align', 'center')
    expect(body.querySelector('strong')).toHaveTextContent('Text')
    expect(body.querySelector('[data-color="red"]')).toHaveTextContent('Text')
    expect(body.querySelector('[data-highlight="yellow"]')).toHaveTextContent('Text')
    await userEvent.click(screen.getByRole('button', { name: '清除格式' }))
    act(() => typeText((body as HTMLElement & { editor: Editor }).editor, ' plain'))
    expect(body.querySelector('h2')).not.toHaveAttribute('data-align')
    expect(body.querySelector('h2')?.lastChild?.nodeType).toBe(Node.TEXT_NODE)
    expect(body.querySelector('h2')?.lastChild?.textContent).toBe(' plain')
    expect(body.querySelector('strong')).toHaveTextContent('Text')
  })

})
