import { act, fireEvent, render, screen } from '@testing-library/react'
import type { Editor } from '@tiptap/core'
import { EditorView } from '@tiptap/pm/view'
import { createRef, StrictMode } from 'react'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import MailBodyEditor, { type MailBodyEditorHandle } from './mail-body-editor'

beforeAll(() => {
  // jsdom has no layout; ProseMirror asks for caret rectangles when scrolling a selection.
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  document.elementFromPoint = () => null
})

const REPLY = '<p></p><p>On 2026-10-25 14:00, Zhang wrote:</p><blockquote><p>Original line</p><blockquote><p>Older</p></blockquote></blockquote>'

async function renderEditor(value = REPLY, fold = true) {
  const onChange = vi.fn()
  const editorRef = createRef<MailBodyEditorHandle>()
  const view = render(<TooltipProvider><MailBodyEditor value={value} onChange={onChange} editorRef={editorRef} foldReplyQuote={fold} /></TooltipProvider>)
  const body = await screen.findByRole('textbox', { name: '正文' })
  const editor = (body as HTMLElement & { editor: Editor }).editor
  const type = (text: string) => act(() => {
    for (const character of text) editor.view.dispatch(editor.state.tr.insertText(character))
  })
  return { onChange, editorRef, body, editor, type, unmount: view.unmount }
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const folded = (body: HTMLElement) => Array.from(body.querySelectorAll('.mail-quote-folded')).map((node) => node.tagName)
const pill = () => screen.queryByRole('button', { name: '显示引用的原文' })

describe('reply quote folding', () => {
  it('hides the last top-level quote and its intro line behind a ··· button', async () => {
    const { body } = await renderEditor()
    expect(folded(body)).toEqual(['P', 'BLOCKQUOTE'])
    // The empty first line stays for the reply itself; the nested quote goes with its parent.
    expect(body.querySelector('p:not(.mail-quote-folded)')).toBeInTheDocument()
    expect(pill()).toBeInTheDocument()
  })

  it('never changes the saved HTML', async () => {
    const plain = await renderEditor(REPLY, false)
    await plain.type('Thanks')
    const expected = plain.onChange.mock.lastCall?.[0]
    plain.unmount()

    const { onChange, type, editor } = await renderEditor()
    await type('Thanks')
    expect(onChange.mock.lastCall?.[0]).toBe(expected)
    expect(expected).toBe(`<p>Thanks</p>${REPLY.slice('<p></p>'.length)}`)
    expect(editor.getHTML()).not.toContain('mail-quote')
  })

  it('expands from the button and stays expanded', async () => {
    const { body, type } = await renderEditor()
    fireEvent.click(pill()!)
    expect(folded(body)).toEqual([])
    expect(pill()).not.toBeInTheDocument()
    await type('more')
    expect(folded(body)).toEqual([])
  })

  it('expands when the selection moves into the folded part', async () => {
    const { body, editorRef } = await renderEditor()
    act(() => editorRef.current?.setSelection({ from: 1, to: 1 }))
    expect(folded(body)).toHaveLength(2)
    act(() => editorRef.current?.setSelection({ from: 4, to: 4 }))
    expect(folded(body)).toEqual([])
  })

  it('steps into the quote with the arrow keys, which the hidden nodes would otherwise skip', async () => {
    const { body, editor } = await renderEditor()
    act(() => editor.commands.setTextSelection(1))
    const event = new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true })
    act(() => { editor.view.dom.dispatchEvent(event) })
    expect(event.defaultPrevented).toBe(true)
    expect(folded(body)).toEqual([])
    expect(editor.state.selection.$head.parent.textContent).toContain('Zhang wrote')
  })

  it('unfolds when an edit changes the quote, so another block never becomes hidden', async () => {
    const { body, editor } = await renderEditor()
    act(() => { editor.view.dispatch(editor.state.tr.delete(2, editor.state.doc.content.size)) })
    expect(folded(body)).toEqual([])
    act(() => { editor.commands.insertContentAt(editor.state.doc.content.size, '<blockquote><p>mine</p></blockquote>') })
    expect(folded(body)).toEqual([])
  })

  it('folds only the quote when the line before it is empty, and nothing without a quote', async () => {
    const { body, unmount } = await renderEditor('<p>Hi</p><p></p><blockquote><p>old</p></blockquote>')
    expect(folded(body)).toEqual(['BLOCKQUOTE'])
    unmount()
    const plain = await renderEditor('<p>Hi</p><p>there</p>')
    expect(folded(plain.body)).toEqual([])
    expect(pill()).not.toBeInTheDocument()
  })

  it('does not fold when the draft is not a reply', async () => {
    const { body } = await renderEditor(REPLY, false)
    expect(folded(body)).toEqual([])
    expect(pill()).not.toBeInTheDocument()
  })

  // StrictMode makes two editors for a moment. They once shared the "···" container, moved it
  // back and forth between their documents, and each re-read and redrew on every move, forever.
  it('settles under StrictMode with the real editor: folds, expands and takes typing, in few transactions', async () => {
    // Count every transaction and every DOM-change callback ProseMirror gets; past the cap the
    // observers are cut off, so a loop fails this test instead of hanging it.
    const CAP = 200
    let count = 0
    const dispatch = EditorView.prototype.dispatch
    vi.spyOn(EditorView.prototype, 'dispatch').mockImplementation(function (this: EditorView, tr) {
      count += 1
      if (count > CAP) throw new Error('editor keeps dispatching')
      return dispatch.call(this, tr)
    })
    const Native = globalThis.MutationObserver
    vi.stubGlobal('MutationObserver', class extends Native {
      constructor(callback: MutationCallback) {
        super((records, observer) => {
          count += 1
          if (count > CAP) observer.disconnect()
          else callback(records, observer)
        })
      }
    })
    const onChange = vi.fn()
    render(<StrictMode><TooltipProvider><MailBodyEditor value={REPLY} onChange={onChange} foldReplyQuote /></TooltipProvider></StrictMode>)
    const body = await screen.findByRole('textbox', { name: '正文' })
    // Let mutation observers and animation frames run.
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 100)) })
    expect(folded(body)).toEqual(['P', 'BLOCKQUOTE'])
    expect(document.querySelectorAll('.mail-quote-fold')).toHaveLength(1)
    const settled = count
    expect(settled).toBeLessThan(20)
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 100)) })
    expect(count).toBe(settled)

    fireEvent.click(pill()!)
    expect(folded(body)).toEqual([])
    const editor = (body as HTMLElement & { editor: Editor }).editor
    act(() => { editor.commands.setTextSelection(1); editor.view.dispatch(editor.state.tr.insertText('Hi')) })
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 100)) })
    expect(onChange.mock.lastCall?.[0]).toBe(`<p>Hi</p>${REPLY.slice('<p></p>'.length)}`)
    expect(count).toBeLessThan(settled + 10)
  })
})

