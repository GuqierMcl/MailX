import { StrictMode } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Draft, DraftAttachmentSummary } from '@/generated/accounts'
import { mockDraftBody } from '@/lib/mock-draft-body'
import { Composer, type ComposerController } from './composer'

beforeAll(() => {
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
})

function attachment(id: string, fileName: string, contentId: string | null): DraftAttachmentSummary {
  return { id, fileName, mediaType: contentId ? 'image/png' : 'application/pdf', size: 4, origin: 'user', inline: contentId !== null, contentId }
}

function setup(bodyHtml = '<p>Hello</p>', attachments: DraftAttachmentSummary[] = []) {
  const account = exampleAccount()
  const client = testClient([account])
  let saved: Draft = {
    id: 'rich-draft', accountId: account.id, kind: 'new', sourceEmailId: null,
    to: [{ address: 'test@example.org', name: null }], cc: [], bcc: [], subject: 'Rich text',
    ...mockDraftBody(bodyHtml), inReplyTo: null, references: null,
    createdAt: '2026-10-24T00:00:00Z', updatedAt: '2026-10-24T00:00:00Z', revision: 1, attachments,
  }
  client.getDraft.mockImplementation(async () => structuredClone(saved))
  client.updateDraft.mockImplementation(async (request) => {
    saved = { ...saved, ...mockDraftBody(request.bodyHtml), revision: saved.revision + 1 }
    return structuredClone(saved)
  })
  client.queueSend.mockResolvedValue({} as never)
  const controller = { current: null as ComposerController | null }
  render(<StrictMode><Composer client={client} draftId={saved.id} accounts={[account]} onSent={() => {}} onDiscarded={() => {}} controllerRef={controller} /></StrictMode>)
  /** Mimics the host adding inline images: bumps the revision and lists them as attachments. */
  const host = (names: string[]) => {
    const inserted = names.map((name, i) => ({ attachmentId: `att-${saved.attachments.length + i}`, contentId: `${name}@x.test`, fileName: `${name}.png` }))
    saved = { ...saved, revision: saved.revision + 1, attachments: [...saved.attachments, ...inserted.map((image) => attachment(image.attachmentId, image.fileName, image.contentId))] }
    return { draft: structuredClone(saved), inserted }
  }
  return { client, controller, host, saved: () => saved }
}

describe('rich editor and composer integration', () => {
  it('mounts lazily under StrictMode, saves formatting-only edits as HTML, and restores the original format', async () => {
    const { client, controller } = setup()
    const body = await screen.findByRole('textbox', { name: '正文' }, { timeout: 5000 })
    expect(body).toHaveTextContent('Hello')
    expect(client.updateDraft).not.toHaveBeenCalled()
    fireEvent.mouseDown(screen.getByRole('button', { name: '无序列表' }))
    fireEvent.click(screen.getByRole('button', { name: '无序列表' }))
    await waitFor(() => expect(body.querySelector('ul li')).toHaveTextContent('Hello'))
    expect(controller.current?.hasChanges()).toBe(true)
    await act(async () => { await controller.current?.flush() })
    expect(client.updateDraft).toHaveBeenLastCalledWith(expect.objectContaining({ bodyHtml: '<ul><li><p>Hello</p></li></ul>' }))
    expect(client.updateDraft.mock.lastCall?.[0]).not.toHaveProperty('bodyText')
    await act(async () => { await controller.current?.restoreBaseline() })
    expect(client.updateDraft).toHaveBeenLastCalledWith(expect.objectContaining({ bodyHtml: '<p>Hello</p>', expectedRevision: 2 }))
  })

  it('keeps Cmd/Ctrl+Enter as two-step send, with no editor line break or list split', async () => {
    const { client } = setup('<ul><li><p>Hello</p></li></ul>')
    const body = await screen.findByRole('textbox', { name: '正文' }, { timeout: 5000 })
    fireEvent.keyDown(body, { key: 'Enter', ctrlKey: true, isComposing: true })
    expect(screen.getByRole('button', { name: '发送' })).toBeInTheDocument()
    fireEvent.keyDown(body, { key: 'Enter', ctrlKey: true })
    await screen.findByRole('button', { name: '确认发送' })
    expect(body.querySelectorAll('li')).toHaveLength(1)
    expect(body.querySelector('br')).toBeNull()
    expect(client.queueSend).not.toHaveBeenCalled()
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 420)) })
    fireEvent.keyDown(body, { key: 'Enter', ctrlKey: true })
    await waitFor(() => expect(client.queueSend).toHaveBeenCalledWith(expect.objectContaining({ expectedRevision: 1 })))
  })

  it('withdraws send confirmation when only formatting changes', async () => {
    const { client } = setup()
    await screen.findByRole('textbox', { name: '正文' }, { timeout: 5000 })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await screen.findByRole('button', { name: '确认发送' })
    fireEvent.click(screen.getByRole('button', { name: '引用' }))
    await screen.findByRole('button', { name: '发送' })
    expect(client.queueSend).not.toHaveBeenCalled()
  })

  it('inserts the images the picker returns at the cursor, then saves the body with them', async () => {
    URL.createObjectURL = vi.fn(() => 'blob:p')
    URL.revokeObjectURL = vi.fn()
    const { client, controller, host, saved } = setup()
    client.inlineImagePreview.mockResolvedValue(new ArrayBuffer(4))
    client.addInlineImages.mockImplementation(async () => host(['one', 'two']))
    const body = await screen.findByRole('textbox', { name: '正文' }, { timeout: 5000 })
    fireEvent.mouseDown(screen.getByRole('button', { name: '插入图片' }))
    fireEvent.click(screen.getByRole('button', { name: '插入图片' }))
    await waitFor(() => expect(body.querySelectorAll('.mail-inline-image')).toHaveLength(2))
    expect(client.addInlineImages).toHaveBeenCalledWith('rich-draft', 1)
    await waitFor(() => expect(body.querySelectorAll('.mail-inline-image img')).toHaveLength(2))
    expect(controller.current?.hasChanges()).toBe(true)
    await act(async () => { await controller.current?.flush() })
    const html = client.updateDraft.mock.lastCall?.[0].bodyHtml ?? ''
    expect(html.indexOf('cid:one@x.test')).toBeGreaterThan(-1)
    expect(html.indexOf('cid:two@x.test')).toBeGreaterThan(html.indexOf('cid:one@x.test'))
    expect(saved().attachments).toHaveLength(2)
  })

  it('stores pasted images one by one, flushing the body between them', async () => {
    const { client, host } = setup()
    client.inlineImagePreview.mockResolvedValue(new ArrayBuffer(4))
    const bodies: string[] = []
    client.addInlineImageBytes.mockImplementation(async (_id, _rev, name) => {
      bodies.push(client.updateDraft.mock.lastCall?.[0].bodyHtml ?? '')
      return host([name.replace('.png', '')])
    })
    URL.createObjectURL = vi.fn(() => 'blob:p')
    URL.revokeObjectURL = vi.fn()
    const body = await screen.findByRole('textbox', { name: '正文' }, { timeout: 5000 })
    const files = ['a.png', 'b.png'].map((name) => new File([new Uint8Array([1, 2, 3])], name, { type: 'image/png' }))
    const event = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(event, 'clipboardData', { value: { files, getData: () => '', types: ['Files'] } })
    await act(async () => { body.dispatchEvent(event) })
    expect(event.defaultPrevented).toBe(true)
    await waitFor(() => expect(body.querySelectorAll('.mail-inline-image')).toHaveLength(2))
    expect(client.addInlineImageBytes).toHaveBeenNthCalledWith(1, 'rich-draft', 1, 'a.png', expect.any(Uint8Array))
    expect(client.addInlineImageBytes).toHaveBeenNthCalledWith(2, 'rich-draft', 3, 'b.png', expect.any(Uint8Array))
    // The second add sees the first image already saved in the body.
    expect(bodies[1]).toContain('cid:a@x.test')
  })

  it('lists only regular attachments in the attachment bar', async () => {
    setup('<p>Hello</p>', [attachment('f', 'contract.pdf', null), attachment('i', 'photo.png', 'p@x.test')])
    await screen.findByRole('textbox', { name: '正文' }, { timeout: 5000 })
    expect(screen.getByText('contract.pdf')).toBeInTheDocument()
    expect(screen.queryByText('photo.png')).not.toBeInTheDocument()
  })

  it('warns, without blocking, that pictures with no attachment will not be sent', async () => {
    const { client } = setup('<p>Hi</p><p><img src="cid:gone@x.test"></p>')
    await screen.findByRole('textbox', { name: '正文' }, { timeout: 5000 })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await screen.findByRole('button', { name: '确认发送' })
    expect(screen.getByText('有 1 张图片已丢失，将不会发出')).toBeInTheDocument()
    expect(client.queueSend).not.toHaveBeenCalled()
  })
})
