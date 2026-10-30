import { createRef } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { Editor } from '@tiptap/core'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import type { DraftAttachmentSummary } from '@/generated/accounts'
import MailBodyEditor, { type MailBodyEditorHandle } from './mail-body-editor'
import { mailEditorOptions } from './mail-body-editor-schema'

const editors: Editor[] = []
beforeAll(() => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  document.elementFromPoint = () => null
})
afterEach(() => { editors.splice(0).forEach((editor) => editor.destroy()) })

function inlineAttachment(id: string, contentId: string): DraftAttachmentSummary {
  return { id, fileName: `${id}.png`, mediaType: 'image/png', size: 4, origin: 'user', inline: true, contentId }
}

describe('inline image node', () => {
  it('parses and renders only cid: images, keeping alt and dimensions', () => {
    const editor = new Editor({ ...mailEditorOptions(), content: '<p>a<img src="cid:one@x.test" alt="Logo" width="40" height="20"><img src="https://tracker.invalid/p.png"><img src="data:image/png;base64,AAAA"><img src="CID:upper"></p>' })
    editors.push(editor)
    expect(editor.getHTML()).toBe('<p>a<img src="cid:one@x.test" alt="Logo" width="40" height="20"></p>')
  })

  it('drops pasted HTML images that are not cid references', () => {
    const editor = new Editor({ ...mailEditorOptions(), content: '<p></p>' })
    editors.push(editor)
    editor.view.pasteHTML('<p>x<img src="https://tracker.invalid/p.png"><img src="data:image/png;base64,AAAA"></p>', new Event('paste') as ClipboardEvent)
    expect(editor.getHTML()).not.toContain('<img')
  })
})

describe('inline image view', () => {
  let revoke: ReturnType<typeof vi.fn>
  let create: ReturnType<typeof vi.fn>
  beforeEach(() => {
    let n = 0
    create = vi.fn(() => `blob:preview-${++n}`)
    revoke = vi.fn()
    URL.createObjectURL = create as unknown as typeof URL.createObjectURL
    URL.revokeObjectURL = revoke as unknown as typeof URL.revokeObjectURL
  })

  function renderEditor(props: Partial<Parameters<typeof MailBodyEditor>[0]> = {}) {
    const onChange = vi.fn()
    const editorRef = createRef<MailBodyEditorHandle>()
    const tree = (extra: typeof props) => (
      <TooltipProvider><MailBodyEditor value="<p>Hi</p>" onChange={onChange} editorRef={editorRef} {...extra} /></TooltipProvider>
    )
    const view = render(tree(props))
    return { onChange, editorRef, unmount: view.unmount, rerender: (extra: typeof props) => view.rerender(tree(extra)) }
  }

  it('shows the preview once per attachment and revokes its URL on unmount', async () => {
    const loadPreview = vi.fn().mockResolvedValue(new ArrayBuffer(4))
    const { unmount } = renderEditor({
      value: '<p><img src="cid:a@x.test" alt="A"><img src="cid:a@x.test" alt="A"></p>',
      inlineImages: { attachments: [inlineAttachment('att-1', 'a@x.test')], loadPreview },
    })
    await waitFor(() => expect(document.querySelectorAll('.mail-inline-image img')).toHaveLength(2))
    expect(loadPreview).toHaveBeenCalledTimes(1)
    expect(loadPreview).toHaveBeenCalledWith('att-1')
    expect(document.querySelector('.mail-inline-image img')).toHaveAttribute('src', 'blob:preview-1')
    expect(revoke).not.toHaveBeenCalled()
    unmount()
    expect(revoke).toHaveBeenCalledWith('blob:preview-1')
  })

  it('revokes the URLs when the window unloads', async () => {
    const loadPreview = vi.fn().mockResolvedValue(new ArrayBuffer(4))
    renderEditor({ value: '<p><img src="cid:a@x.test"></p>', inlineImages: { attachments: [inlineAttachment('att-1', 'a@x.test')], loadPreview } })
    await waitFor(() => expect(document.querySelector('.mail-inline-image img')).not.toBeNull())
    window.dispatchEvent(new Event('pagehide'))
    expect(revoke).toHaveBeenCalledWith('blob:preview-1')
  })

  it('shows a placeholder when no inline attachment has the Content-ID', () => {
    const loadPreview = vi.fn()
    renderEditor({ value: '<p><img src="cid:gone@x.test"></p>', inlineImages: { attachments: [], loadPreview } })
    expect(screen.getByRole('img', { name: '图片已丢失' })).toHaveTextContent('图片已丢失')
    expect(loadPreview).not.toHaveBeenCalled()
  })

  it('shows the placeholder when the preview is notFound, and an image once the attachment appears', async () => {
    const loadPreview = vi.fn().mockRejectedValue({ code: 'notFound' })
    const { rerender } = renderEditor({ value: '<p><img src="cid:a@x.test"></p>', inlineImages: { attachments: [inlineAttachment('att-1', 'a@x.test')], loadPreview } })
    await screen.findByRole('img', { name: '图片已丢失' })
    loadPreview.mockResolvedValue(new ArrayBuffer(4))
    rerender({ value: '<p><img src="cid:a@x.test"></p>', inlineImages: { attachments: [inlineAttachment('att-2', 'a@x.test')], loadPreview } })
    await waitFor(() => expect(document.querySelector('.mail-inline-image img')).not.toBeNull())
  })

  it('hands pasted image files to the host and lets other pastes through', () => {
    const onPasteImages = vi.fn()
    renderEditor({ onPasteImages })
    const body = screen.getByRole('textbox', { name: '正文' })
    const png = new File([new Uint8Array([1, 2])], 'shot.png', { type: 'image/png' })
    const paste = (files: File[]) => {
      const event = new Event('paste', { bubbles: true, cancelable: true })
      Object.defineProperty(event, 'clipboardData', { value: { files, getData: () => '', types: files.length ? ['Files'] : [] } })
      body.dispatchEvent(event)
      return event
    }
    const withImage = paste([png, new File(['x'], 'a.txt', { type: 'text/plain' })])
    expect(withImage.defaultPrevented).toBe(true)
    expect(onPasteImages).toHaveBeenCalledWith([png])
    onPasteImages.mockClear()
    paste([new File(['x'], 'a.txt', { type: 'text/plain' })])
    expect(onPasteImages).not.toHaveBeenCalled()
  })

  it('shows the toolbar button only when wired, disabled when read-only, and inserts nodes at the cursor in order', async () => {
    const onInsertImage = vi.fn()
    const { editorRef, onChange, rerender } = renderEditor({ onInsertImage })
    const button = await screen.findByRole('button', { name: '插入图片' })
    fireEvent.click(button)
    expect(onInsertImage).toHaveBeenCalledTimes(1)
    act(() => editorRef.current?.insertInlineImages([
      { attachmentId: 'att-1', contentId: 'a@x.test', fileName: 'a.png' },
      { attachmentId: 'att-2', contentId: 'b@x.test', fileName: 'b.png' },
    ]))
    const html = String(onChange.mock.lastCall?.[0])
    expect(html.indexOf('cid:a@x.test')).toBeGreaterThan(-1)
    expect(html.indexOf('cid:b@x.test')).toBeGreaterThan(html.indexOf('cid:a@x.test'))
    rerender({ onInsertImage, readOnly: true })
    expect(screen.getByRole('button', { name: '插入图片' })).toBeDisabled()
  })
})
