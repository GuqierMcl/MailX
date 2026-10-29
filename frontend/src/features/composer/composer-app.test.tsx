import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { fireEvent } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Account, Draft } from '@/generated/accounts'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { TitleBarSlotsProvider } from '@/features/window-chrome/title-bar'
import { ComposerApp } from './composer-app'
import { AUTOSAVE_DELAY_MS } from './use-draft-editor'


// The real editing engine is covered by mail-body-editor and composer-rich-text tests.
vi.mock('./mail-body-editor', () => ({
  default: ({ value, onChange, readOnly }: { value: string; onChange: (html: string) => void; readOnly?: boolean }) => (
    <textarea aria-label="正文" value={value} disabled={readOnly} onChange={(event) => onChange(event.target.value)} />
  ),
}))

const DRAFT_ID = '3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b'
const account = exampleAccount()

function draft(overrides: Partial<Draft> = {}): Draft {
  return {
    id: DRAFT_ID, accountId: account.id, kind: 'new', sourceEmailId: null, to: [], cc: [], bcc: [], subject: '', bodyText: overrides.bodyHtml?.replace(/<[^>]+>/g, '') ?? '', bodyHtml: '',
    inReplyTo: null, references: null, createdAt: '2026-10-23T00:00:00Z', updatedAt: '2026-10-23T00:00:00Z', revision: 1, ...overrides,
  }
}

function setup(initial: Draft = draft(), fresh = false, rows: Account[] = [account]) {
  const client = testClient(rows)
  let current = initial
  client.getDraft.mockImplementation(async () => structuredClone(current))
  client.updateDraft.mockImplementation(async (request) => {
    current = { ...current, to: request.to, cc: request.cc, bcc: request.bcc, subject: request.subject, bodyHtml: request.bodyHtml, bodyText: request.bodyHtml.replace(/<[^>]+>/g, ''), revision: current.revision + 1 }
    return structuredClone(current)
  })
  let generation = 0
  const handlers = { close: () => undefined as void, quit: () => undefined as void, cancel: (value: number) => { void value } }
  client.onComposerCloseRequested.mockImplementation((listener) => { handlers.close = () => listener({ generation: ++generation }); return () => undefined })
  client.onAppClosing.mockImplementation((listener) => { handlers.quit = () => listener({ generation: ++generation }); return () => undefined })
  client.onAppCloseCancelled.mockImplementation((listener) => { handlers.cancel = (value) => listener({ generation: value }); return () => undefined })
  client.cancelComposerClose.mockImplementation(async (value) => { handlers.cancel(value) })
  const removalListeners = new Set<Parameters<MailxClient['onAccountRemoved']>[0]>()
  client.onAccountRemoved.mockImplementation((listener) => { removalListeners.add(listener); return () => { removalListeners.delete(listener) } })
  const removeAccount = (draftIds = [DRAFT_ID]) => act(() => {
    removalListeners.forEach((listener) => listener({ accountId: account.id, draftIds, credentialsPending: false }))
  })
  render(<TitleBarSlotsProvider><ComposerApp client={client} draftId={DRAFT_ID} fresh={fresh} host={null} /></TitleBarSlotsProvider>)
  return { client, handlers, removeAccount }
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
    const { client, handlers } = setup(draft({ subject: '周会', bodyHtml: '内容' }))
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
    expect(client.cancelComposerClose).toHaveBeenCalledWith(1)

    await act(async () => { handlers.close() })
    await userEvent.click(await screen.findByRole('button', { name: '保存草稿' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.updateDraft).toHaveBeenCalledWith(expect.objectContaining({ bodyHtml: '还没保存的正文' }))
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
    const { client, handlers } = setup(draft({ subject: '周会', bodyHtml: '原文' }))
    fireEvent.change(await body(), { target: { value: '改过的' } })
    await act(async () => { handlers.close() })
    await userEvent.click(await screen.findByRole('button', { name: '不保存' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalled())
    expect(client.deleteDraft).not.toHaveBeenCalled()
    expect(client.updateDraft).toHaveBeenLastCalledWith(expect.objectContaining({ subject: '周会', bodyHtml: '原文' }))
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

  it('cancels application quit on a failed save and lets the user retry safely', async () => {
    const { client, handlers } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'x' } })
    await act(async () => { handlers.quit() })
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('已取消退出 MailX')
    expect(client.cancelComposerClose).toHaveBeenCalledWith(1)
    expect(client.closeComposer).not.toHaveBeenCalled()
    client.updateDraft.mockResolvedValue(draft({ bodyHtml: 'x', revision: 2 }))
    await userEvent.click(within(dialog).getByRole('button', { name: '重试' }))
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalledWith(false, undefined))
  })

  it('ignores a late save acknowledgement after application quit was cancelled', async () => {
    const { client, handlers } = setup()
    fireEvent.change(await body(), { target: { value: 'x' } })
    let resolveSave!: (saved: Draft) => void
    client.updateDraft.mockReturnValueOnce(new Promise((resolve) => { resolveSave = resolve }))
    await act(async () => { handlers.quit() })
    await waitFor(() => expect(client.updateDraft).toHaveBeenCalled())
    act(() => { handlers.cancel(1) })
    await act(async () => { resolveSave(draft({ bodyHtml: 'x', revision: 2 })) })
    expect(client.closeComposer).not.toHaveBeenCalled()
    await act(async () => { handlers.quit() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalledExactlyOnceWith(false, 2))
  })

  it('does not delete an emptied draft when its delayed quit save finishes after cancellation', async () => {
    const { client, handlers } = setup(draft({ bodyHtml: 'original' }))
    fireEvent.change(await body(), { target: { value: '' } })
    let resolveSave!: (saved: Draft) => void
    client.updateDraft.mockReturnValueOnce(new Promise((resolve) => { resolveSave = resolve }))
    await act(async () => { handlers.quit() })
    await waitFor(() => expect(client.updateDraft).toHaveBeenCalled())
    act(() => { handlers.cancel(1) })
    await act(async () => { resolveSave(draft({ bodyHtml: '', revision: 2 })) })
    expect(client.deleteDraft).not.toHaveBeenCalled()
    expect(client.closeComposer).not.toHaveBeenCalled()
    expect(await body()).toBeEnabled()
  })

  it('keeps an empty draft durable during cancellable application quit', async () => {
    const { client, handlers } = setup()
    await body()
    await act(async () => { handlers.quit() })
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalledWith(false, 1))
    expect(client.deleteDraft).not.toHaveBeenCalled()
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

  it('force closes an account-removed draft without flushing, deleting or asking', async () => {
    const { client, handlers, removeAccount } = setup(draft({ subject: 'Original' }))
    fireEvent.change(await body(), { target: { value: 'Unsaved edits' } })
    await act(async () => { handlers.close() })
    expect(await screen.findByRole('alertdialog')).toBeInTheDocument()
    removeAccount()
    await waitFor(() => expect(client.closeComposer).toHaveBeenCalledWith(true))
    await act(async () => { handlers.close(); handlers.quit() })
    expect(client.closeComposer).toHaveBeenCalledTimes(1)
    expect(client.updateDraft).not.toHaveBeenCalled()
    expect(client.deleteDraft).not.toHaveBeenCalled()
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(screen.queryByRole('textbox', { name: '正文' })).not.toBeInTheDocument()
  })

  it('refreshes the sender list without closing a draft absent from the deletion report', async () => {
    const other = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', emailAddress: 'other@example.com', displayName: '个人邮箱' })
    const { client, removeAccount } = setup(draft({ accountId: other.id }), false, [account, other])
    await body()
    client.listAccounts.mockResolvedValue([other])
    removeAccount([])
    await waitFor(() => expect(client.listAccounts).toHaveBeenCalledTimes(2))
    expect(client.closeComposer).not.toHaveBeenCalled()
    expect(screen.getByRole('textbox', { name: '正文' })).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: '发件人' })).toHaveTextContent('other@example.com')
  })

  it('ignores a late queue success after the account removal already closed its composer', async () => {
    const { client, removeAccount } = setup(draft({ to: [{ address: 'li@example.org', name: null }], subject: 'Hello' }))
    let resolveQueue!: (item: Awaited<ReturnType<MailxClient['queueSend']>>) => void
    client.queueSend.mockReturnValue(new Promise((resolve) => { resolveQueue = resolve }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await screen.findByRole('button', { name: '确认发送' })
    await new Promise((resolve) => setTimeout(resolve, 450))
    await userEvent.click(screen.getByRole('button', { name: '确认发送' }))
    expect(client.queueSend).toHaveBeenCalledTimes(1)
    removeAccount()
    await act(async () => resolveQueue({} as never))
    expect(client.closeComposer).toHaveBeenCalledExactlyOnceWith(true)
    expect(client.updateDraft).not.toHaveBeenCalled()
  })
})
