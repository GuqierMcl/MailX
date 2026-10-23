import { act, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { fireEvent } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Draft } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { TitleBarSlotsProvider } from '@/features/window-chrome/title-bar'
import { ComposerApp } from './composer-app'
import { AUTOSAVE_DELAY_MS } from './use-draft-editor'

const DRAFT_ID = '3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b'
const account = exampleAccount()

function draft(overrides: Partial<Draft> = {}): Draft {
  return {
    id: DRAFT_ID, accountId: account.id, kind: 'new', sourceEmailId: null, to: [], cc: [], bcc: [], subject: '', bodyText: '',
    inReplyTo: null, references: null, createdAt: '2026-10-23T00:00:00Z', updatedAt: '2026-10-23T00:00:00Z', revision: 1, ...overrides,
  }
}

function setup(initial: Draft = draft()) {
  const client = testClient([account])
  let current = initial
  client.getDraft.mockImplementation(async () => structuredClone(current))
  client.updateDraft.mockImplementation(async (request) => {
    current = { ...current, to: request.to, cc: request.cc, bcc: request.bcc, subject: request.subject, bodyText: request.bodyText, revision: current.revision + 1 }
    return structuredClone(current)
  })
  const handlers: { close: () => void; quit: () => void } = { close: () => undefined, quit: () => undefined }
  client.onComposerCloseRequested.mockImplementation((listener) => { handlers.close = listener; return () => undefined })
  client.onAppClosing.mockImplementation((listener) => { handlers.quit = listener; return () => undefined })
  render(<TitleBarSlotsProvider><ComposerApp client={client} draftId={DRAFT_ID} host={null} /></TitleBarSlotsProvider>)
  return { client, handlers }
}

const body = () => screen.findByRole('textbox', { name: '正文' })

afterEach(() => { document.title = '' })

describe('composer window', () => {
  it('titles the window with the subject, or 新邮件 while it is empty', async () => {
    setup()
    await body()
    expect(document.title).toBe('新邮件')
    expect(screen.getByRole('banner')).toHaveTextContent('新邮件')
    await userEvent.type(screen.getByRole('textbox', { name: '主题' }), '周会')
    expect(document.title).toBe('周会')
    expect(screen.getByRole('banner')).toHaveTextContent('周会')
  })

  it('renders inside the shared canvas with one island and the existing title bar', async () => {
    setup()
    await body()
    expect(document.querySelector('.app-canvas')).not.toBeNull()
    expect(document.querySelectorAll('[data-slot="island"]')).toHaveLength(1)
    expect(document.querySelectorAll('[data-slot="title-bar"]')).toHaveLength(1)
  })

  it('on a close request deletes a completely empty draft, then closes', async () => {
    const { client, handlers } = setup()
    await body()
    await act(async () => { handlers.close() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.deleteDraft).toHaveBeenCalledWith(DRAFT_ID)
  })

  it('flushes a pending edit before closing and keeps a draft that has content', async () => {
    const { client, handlers } = setup()
    const text = await body()
    fireEvent.change(text, { target: { value: '还没保存的正文' } })
    expect(client.updateDraft).not.toHaveBeenCalled()
    await act(async () => { handlers.close() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.updateDraft).toHaveBeenCalledWith(expect.objectContaining({ bodyText: '还没保存的正文' }))
    expect(client.deleteDraft).not.toHaveBeenCalled()
    expect(AUTOSAVE_DELAY_MS).toBeGreaterThan(0)
  })

  it('asks before closing when the last save failed; cancel keeps the window, confirm closes', async () => {
    const { client, handlers } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'x' } })
    await act(async () => { handlers.close() })
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('仍要关闭（丢失未保存的修改）')
    expect(client.closeComposer).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(client.closeComposer).not.toHaveBeenCalled()

    await act(async () => { handlers.close() })
    await userEvent.click(await screen.findByRole('button', { name: '仍要关闭（丢失未保存的修改）' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalledTimes(1))
    expect(client.deleteDraft).not.toHaveBeenCalled()
  })

  it('closes without asking when the whole app is quitting, even if the save failed', async () => {
    const { client, handlers } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'x' } })
    await act(async () => { handlers.quit() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
  })

  it('closes right after the second send click succeeded', async () => {
    const { client } = setup()
    client.queueSend.mockResolvedValue({} as never)
    await body()
    await userEvent.type(screen.getByRole('textbox', { name: '收件人' }), 'li@example.org{Enter}')
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await screen.findByRole('button', { name: '无主题，确认发送' })
    await new Promise((resolve) => setTimeout(resolve, 450))
    await userEvent.click(screen.getByRole('button', { name: '无主题，确认发送' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.queueSend).toHaveBeenCalledTimes(1)
  })

  it('does not delete a draft the queue holds when closing', async () => {
    const { client, handlers } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValueOnce(new MailxClientError({ code: 'draftLocked', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'x' } })
    await act(async () => { handlers.close() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.deleteDraft).not.toHaveBeenCalled()
  })
})
