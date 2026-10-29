import { StrictMode } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import { beforeAll, describe, expect, it } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Draft } from '@/generated/accounts'
import { mockDraftBody } from '@/lib/mock-draft-body'
import { Composer, type ComposerController } from './composer'

beforeAll(() => {
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
})

function setup(bodyHtml = '<p>Hello</p>') {
  const account = exampleAccount()
  const client = testClient([account])
  let saved: Draft = {
    id: 'rich-draft', accountId: account.id, kind: 'new', sourceEmailId: null,
    to: [{ address: 'test@example.org', name: null }], cc: [], bcc: [], subject: 'Rich text',
    ...mockDraftBody(bodyHtml), inReplyTo: null, references: null,
    createdAt: '2026-10-24T00:00:00Z', updatedAt: '2026-10-24T00:00:00Z', revision: 1, attachments: [],
  }
  client.getDraft.mockImplementation(async () => structuredClone(saved))
  client.updateDraft.mockImplementation(async (request) => {
    saved = { ...saved, ...mockDraftBody(request.bodyHtml), revision: saved.revision + 1 }
    return structuredClone(saved)
  })
  client.queueSend.mockResolvedValue({} as never)
  const controller = { current: null as ComposerController | null }
  render(<StrictMode><Composer client={client} draftId={saved.id} accounts={[account]} onSent={() => {}} onDiscarded={() => {}} controllerRef={controller} /></StrictMode>)
  return { client, controller }
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
})
