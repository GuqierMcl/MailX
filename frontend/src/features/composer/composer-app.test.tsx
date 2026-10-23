import { act, render, screen, waitFor, within } from '@/test/render'
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

function setup(initial: Draft = draft(), fresh = false) {
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
  render(<TitleBarSlotsProvider><ComposerApp client={client} draftId={DRAFT_ID} fresh={fresh} host={null} /></TitleBarSlotsProvider>)
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

  it('on a close request deletes a blank draft without asking', async () => {
    const { client, handlers } = setup(draft(), true)
    await body()
    await act(async () => { handlers.close() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.deleteDraft).toHaveBeenCalledWith(DRAFT_ID)
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
  })

  it('closes an existing draft that was not changed without asking or writing', async () => {
    const { client, handlers } = setup(draft({ subject: '周会', bodyText: '内容' }))
    await body()
    await act(async () => { handlers.close() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(client.deleteDraft).not.toHaveBeenCalled()
    expect(client.updateDraft).not.toHaveBeenCalled()
  })

  it('asks when a changed draft is closed; 取消 keeps the window, 保存草稿 flushes and closes', async () => {
    const { client, handlers } = setup(draft({ subject: '周会' }))
    const text = await body()
    fireEvent.change(text, { target: { value: '还没保存的正文' } })
    await act(async () => { handlers.close() })
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('要保存这封草稿吗？')
    expect(dialog).toHaveTextContent('保存后可以在“草稿”中继续编辑。')
    expect(screen.getByRole('button', { name: '保存草稿' })).toHaveFocus()
    const names = within(dialog).getAllByRole('button').map((button) => button.textContent)
    expect(names).toEqual(['不保存', '取消', '保存草稿'])
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(client.closeComposer).not.toHaveBeenCalled()

    await act(async () => { handlers.close() })
    await userEvent.click(await screen.findByRole('button', { name: '保存草稿' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.updateDraft).toHaveBeenCalledWith(expect.objectContaining({ bodyText: '还没保存的正文' }))
    expect(client.deleteDraft).not.toHaveBeenCalled()
    expect(AUTOSAVE_DELAY_MS).toBeGreaterThan(0)
  })

  it('asks for a fresh draft with content even when it was already autosaved; 不保存 deletes it', async () => {
    const { client, handlers } = setup(draft(), true)
    fireEvent.change(await body(), { target: { value: 'x' } })
    await act(async () => { handlers.close() })
    await userEvent.click(await screen.findByRole('button', { name: '不保存' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.deleteDraft).toHaveBeenCalledWith(DRAFT_ID)
  })

  it('不保存 on an existing draft restores the opening content and never deletes it', async () => {
    const { client, handlers } = setup(draft({ subject: '周会', bodyText: '原文' }))
    fireEvent.change(await body(), { target: { value: '改过的' } })
    await act(async () => { handlers.close() })
    await userEvent.click(await screen.findByRole('button', { name: '不保存' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.deleteDraft).not.toHaveBeenCalled()
    expect(client.updateDraft).toHaveBeenLastCalledWith(expect.objectContaining({ subject: '周会', bodyText: '原文' }))
  })

  it('asks again with the failure dialog when 保存草稿 fails; cancel keeps the window, confirm closes', async () => {
    const { client, handlers } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'x' } })
    await act(async () => { handlers.close() })
    await userEvent.click(await screen.findByRole('button', { name: '保存草稿' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('仍要关闭（丢失未保存的修改）')
    expect(client.closeComposer).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(client.closeComposer).not.toHaveBeenCalled()

    await act(async () => { handlers.close() })
    await userEvent.click(await screen.findByRole('button', { name: '保存草稿' }))
    await userEvent.click(await screen.findByRole('button', { name: '仍要关闭（丢失未保存的修改）' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalledTimes(1))
    expect(client.deleteDraft).not.toHaveBeenCalled()
  })

  it('puts the save status before the actions and 发送 last', async () => {
    setup()
    await body()
    const status = document.querySelector('[data-status]') as HTMLElement
    const discard = screen.getByRole('button', { name: '丢弃' })
    const send = screen.getByRole('button', { name: '发送' })
    expect(status.compareDocumentPosition(discard) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(discard.compareDocumentPosition(send) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('closes without asking when the whole app is quitting, even if the save failed', async () => {
    const { client, handlers } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'x' } })
    await act(async () => { handlers.quit() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(client.updateDraft).toHaveBeenCalled()
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
    await userEvent.click(await screen.findByRole('button', { name: '保存草稿' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.deleteDraft).not.toHaveBeenCalled()
  })
})
