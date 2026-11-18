import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Toaster } from '@/components/ui/toast'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Workbench } from '@/features/mail/workbench'
import type { ComposeOpenMode, Draft, DraftSummary, EmailDetail, EmailSummary, Folder, OutboxItem, ReadingMode } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { withTitleBar } from '@/test/title-bar'

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, convertFileSrc: () => 'mailx-body://localhost/' }))
// The real editing engine is covered by the composer's own tests.
vi.mock('@/features/composer/mail-body-editor', () => ({
  default: ({ value, onChange, readOnly }: { value: string; onChange: (html: string) => void; readOnly?: boolean }) => (
    <textarea aria-label="正文" value={value} disabled={readOnly} onChange={(event) => onChange(event.target.value)} />
  ),
}))

const account = exampleAccount()
const inbox: Folder = { id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox', selectable: true, countsKnown: true, unreadCount: 0, totalCount: 2, archivedCount: 0, archivedUnreadCount: 0 }
function mail(id: string, subject: string): EmailSummary {
  return {
    senderGroup: null,
    id, accountId: account.id, folderId: 'inbox', subject, from: { name: '陈晓', address: 'chen@example.com' }, preview: '',
    receivedAt: '2026-10-18T08:00:00.000Z', isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false,
  }
}
const mails = [mail('mail-1', '季度计划'), mail('mail-2', '周会纪要')]
const detail = (summary: EmailSummary): EmailDetail => ({ summary, to: [], cc: [], replyTo: [], body: { text: '正文', hasHtml: false, truncated: false, remoteImagesBlocked: 0 }, bodyFailure: null, attachments: [] })

const NEW_ID = '11111111-1111-4111-8111-111111111111'
const REPLY_ID = '22222222-2222-4222-8222-222222222222'
const SAVED_ID = '3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b'

function draft(id: string, overrides: Partial<Draft> = {}): Draft {
  return {
    id, accountId: account.id, kind: 'new', origin: 'user', sourceEmailId: null, to: [], cc: [], bcc: [], subject: '', bodyHtml: '', bodyText: '',
    inReplyTo: null, references: null, createdAt: '2026-10-23T00:00:00Z', updatedAt: '2026-10-23T00:00:00Z', revision: 1, attachments: [], ...overrides,
  }
}
const summary = (d: Draft): DraftSummary => ({ id: d.id, accountId: d.accountId, kind: d.kind, origin: d.origin, sourceEmailId: null, to: d.to, subject: d.subject, preview: d.bodyText, updatedAt: d.updatedAt, revision: d.revision })

async function setup({ mode = 'inline', saved = [], readingMode = 'split' }: { mode?: ComposeOpenMode; saved?: Draft[]; readingMode?: ReadingMode } = {}) {
  const client = testClient([account])
  client.listFolders.mockResolvedValue([inbox])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState('inbox')] })])
  client.listEmails.mockResolvedValue({ items: mails, nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } })
  client.getEmail.mockImplementation(async (id) => detail(mails.find((row) => row.id === id)!))
  const drafts = new Map<string, Draft>(saved.map((row) => [row.id, row]))
  client.createDraft.mockImplementation(async () => { drafts.set(NEW_ID, draft(NEW_ID)); return structuredClone(drafts.get(NEW_ID)!) })
  client.createDraftFrom.mockImplementation(async () => {
    drafts.set(REPLY_ID, draft(REPLY_ID, { kind: 'reply', to: [{ name: '陈晓', address: 'chen@example.com' }], subject: '回复：季度计划' }))
    return structuredClone(drafts.get(REPLY_ID)!)
  })
  client.listDrafts.mockImplementation(async () => [...drafts.values()].map(summary))
  client.getDraft.mockImplementation(async (id) => {
    const row = drafts.get(id)
    if (!row) throw new MailxClientError({ code: 'notFound', message: 'x', field: null })
    return structuredClone(row)
  })
  client.updateDraft.mockImplementation(async (request) => {
    const row = drafts.get(request.id)!
    const next = { ...row, to: request.to, cc: request.cc, bcc: request.bcc, subject: request.subject, bodyHtml: request.bodyHtml, bodyText: request.bodyHtml.replace(/<[^>]+>/g, ''), revision: row.revision + 1 }
    drafts.set(request.id, next)
    return structuredClone(next)
  })
  client.deleteDraft.mockImplementation(async (id) => { drafts.delete(id) })
  await client.updateSettings({ compose: { defaultAccountId: '', undoSendSeconds: 0, openMode: mode }, layout: { readingMode } })
  client.updateSettings.mockClear()
  const user = userEvent.setup()
  render(withTitleBar(<><Toaster /><Workbench client={client} /></>))
  await screen.findByRole('list', { name: '邮件列表' })
  return { client, user, drafts }
}

const composer = () => screen.findByRole('region', { name: '写邮件' })
const noComposer = () => expect(screen.queryByRole('region', { name: '写邮件' })).not.toBeInTheDocument()
const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
const composeButton = () => screen.getAllByRole('button', { name: '写邮件' })[0]
/**
 * Focus, then type: in jsdom every box is at 0,0, so the panel group takes a pointer
 * press anywhere for its resize handle and would steal the focus a click gives.
 */
async function typeInto(element: HTMLElement, text: string) {
  act(() => element.focus())
  await userEvent.keyboard(text)
}
/** A mouse Shift-click: keyboard activation never flips the mode. */
const shiftClick = (element: Element) => fireEvent.click(element, { shiftKey: true, detail: 1 })

afterEach(() => vi.restoreAllMocks())

describe('where compose opens', () => {
  it('opens inline in the reader when the setting says so, and no window', async () => {
    const { client } = await setup()
    await userEvent.click(composeButton())
    const region = await composer()
    expect(within(region).getByRole('textbox', { name: '主题' })).toBeInTheDocument()
    expect(within(region).getByRole('button', { name: '在新窗口中打开' })).toBeInTheDocument()
    expect(within(region).getByRole('button', { name: '关闭' })).toBeInTheDocument()
    expect(client.openComposer).not.toHaveBeenCalled()
  })

  it('Shift-click flips the setting for that one open', async () => {
    const { client } = await setup({ mode: 'window' })
    shiftClick(composeButton())
    await composer()
    expect(client.openComposer).not.toHaveBeenCalled()
  })

  it('Shift-click on an inline setting opens a window instead', async () => {
    const { client } = await setup()
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    shiftClick(screen.getByRole('button', { name: '回复' }))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith(REPLY_ID, true))
    noComposer()
  })

  it('keyboard opens follow the setting even with Shift held on the button', async () => {
    const { client } = await setup()
    // Enter on the focused button: `detail` is 0, so no flip.
    fireEvent.click(composeButton(), { shiftKey: true, detail: 0 })
    await composer()
    expect(client.openComposer).not.toHaveBeenCalled()
  })

  it('a reply opens inline with the reply draft', async () => {
    const { client } = await setup()
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    await userEvent.click(screen.getByRole('button', { name: '回复' }))
    const region = await composer()
    expect(client.createDraftFrom).toHaveBeenCalledWith(expect.objectContaining({ emailId: 'mail-1', kind: 'reply' }))
    expect(await within(region).findByDisplayValue('回复：季度计划')).toBeInTheDocument()
  })
})

describe('leaving the inline composer', () => {
  it('deletes a blank new draft quietly when another message is opened', async () => {
    const { client } = await setup()
    await userEvent.click(composeButton())
    await composer()
    await userEvent.click(rows()[1])
    await screen.findByRole('heading', { level: 2, name: '周会纪要' })
    noComposer()
    expect(client.deleteDraft).toHaveBeenCalledWith(NEW_ID)
    expect(screen.queryByText('已存为草稿')).not.toBeInTheDocument()
  })

  it('saves anything else, says 已存为草稿, and 继续编辑 reopens it', async () => {
    const { client, drafts } = await setup()
    await userEvent.click(composeButton())
    const region = await composer()
    await typeInto(within(region).getByRole('textbox', { name: '主题' }), '预算')
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    noComposer()
    expect(drafts.get(NEW_ID)?.subject).toBe('预算')
    expect(client.deleteDraft).not.toHaveBeenCalled()
    expect(await screen.findByText('已存为草稿')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '继续编辑' }))
    expect(await within(await composer()).findByDisplayValue('预算')).toBeInTheDocument()
    expect(client.focusComposer).toHaveBeenCalledWith(NEW_ID)
  })

  it('closes with 关闭 the same way, and Esc does not close it', async () => {
    const { user, drafts } = await setup()
    await userEvent.click(composeButton())
    const region = await composer()
    await typeInto(within(region).getByRole('textbox', { name: '主题' }), '预算')
    await user.keyboard('{Escape}')
    expect(screen.getByRole('region', { name: '写邮件' })).toBeInTheDocument()
    await user.click(within(region).getByRole('button', { name: '关闭' }))
    await waitFor(noComposer)
    expect(drafts.get(NEW_ID)?.subject).toBe('预算')
  })

  it('keeps the single-pane composer on screen until the draft can be saved', async () => {
    const { client, user, drafts } = await setup({ readingMode: 'single' })
    await user.click(composeButton())
    const region = await composer()
    const subject = within(region).getByRole('textbox', { name: '主题' })
    const close = within(region).getByRole('button', { name: '关闭' })
    expect(screen.queryByRole('list', { name: '邮件列表' })).not.toBeInTheDocument()
    await typeInto(subject, '预算')
    client.updateDraft.mockRejectedValueOnce(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))

    await user.click(close)
    expect((await screen.findAllByText('草稿没有保存')).length).toBeGreaterThan(0)
    expect(subject).toHaveValue('预算')
    expect(screen.queryByRole('list', { name: '邮件列表' })).not.toBeInTheDocument()

    await waitFor(() => expect(close).toBeEnabled())
    await user.click(close)
    expect(await screen.findByRole('list', { name: '邮件列表' })).toBeInTheDocument()
    noComposer()
    expect(drafts.get(NEW_ID)?.subject).toBe('预算')
  })

  it('switching mailbox saves first', async () => {
    const { drafts } = await setup()
    await userEvent.click(composeButton())
    const region = await composer()
    await typeInto(within(region).getByRole('textbox', { name: '主题' }), '预算')
    await userEvent.click(screen.getByRole('button', { name: /^草稿/ }))
    await waitFor(noComposer)
    expect(drafts.get(NEW_ID)?.subject).toBe('预算')
  })

  it('keeps the composer open with the failure when saving fails', async () => {
    const { client, drafts } = await setup()
    await userEvent.click(composeButton())
    const region = await composer()
    client.updateDraft.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))
    await typeInto(within(region).getByRole('textbox', { name: '主题' }), '预算')
    await userEvent.click(rows()[1])
    expect((await screen.findAllByText('草稿没有保存')).length).toBeGreaterThan(0)
    expect(screen.getByRole('region', { name: '写邮件' })).toBeInTheDocument()
    expect(within(region).getByRole('textbox', { name: '主题' })).toHaveValue('预算')
    expect(within(region).getByText('保存失败')).toBeInTheDocument()
    expect(drafts.has(NEW_ID)).toBe(true)
    expect(client.deleteDraft).not.toHaveBeenCalled()
  })

  it('opening another compose saves the current one first', async () => {
    const { client, drafts } = await setup()
    await userEvent.click(composeButton())
    const region = await composer()
    await typeInto(within(region).getByRole('textbox', { name: '主题' }), '预算')
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    await userEvent.click(screen.getByRole('button', { name: '回复' }))
    expect(await within(await composer()).findByDisplayValue('回复：季度计划')).toBeInTheDocument()
    expect(drafts.get(NEW_ID)?.subject).toBe('预算')
    expect(client.openComposer).not.toHaveBeenCalled()
  })

  it('Cmd+N with the composer open saves it and opens a new one', async () => {
    const { client, user } = await setup()
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    await userEvent.click(screen.getByRole('button', { name: '回复' }))
    const region = await composer()
    act(() => within(region).getByRole('textbox', { name: '主题' }).focus())
    await user.keyboard('{Meta>}n{/Meta}')
    await waitFor(() => expect(client.createDraft).toHaveBeenCalledTimes(1))
    expect(await within(await composer()).findByRole('textbox', { name: '主题' })).toHaveValue('')
  })
})

describe('one draft, one editor', () => {
  it('在新窗口中打开 saves, opens the window with fresh, and closes inline', async () => {
    const { client, drafts } = await setup()
    await userEvent.click(composeButton())
    const region = await composer()
    await typeInto(within(region).getByRole('textbox', { name: '主题' }), '预算')
    await userEvent.click(within(region).getByRole('button', { name: '在新窗口中打开' }))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith(NEW_ID, true))
    expect(drafts.get(NEW_ID)?.subject).toBe('预算')
    await waitFor(noComposer)
    expect(client.deleteDraft).not.toHaveBeenCalled()
  })

  it('继续编辑 focuses a composer window that already has the draft', async () => {
    const saved = draft(SAVED_ID, { to: [{ name: null, address: 'li@example.org' }], subject: '草稿主题', bodyHtml: '<p>草稿正文</p>', bodyText: '草稿正文', revision: 2 })
    const { client } = await setup({ saved: [saved] })
    client.focusComposer.mockResolvedValue(true)
    await userEvent.click(screen.getByRole('button', { name: /^草稿/ }))
    await userEvent.click(await screen.findByText('草稿主题'))
    await userEvent.click(await screen.findByRole('button', { name: '继续编辑' }))
    await waitFor(() => expect(client.focusComposer).toHaveBeenCalledWith(SAVED_ID))
    noComposer()
    expect(client.openComposer).not.toHaveBeenCalled()
  })

  it('继续编辑 opens inline when no window has it; Shift-click opens the window', async () => {
    const saved = draft(SAVED_ID, { to: [{ name: null, address: 'li@example.org' }], subject: '草稿主题', bodyHtml: '<p>草稿正文</p>', bodyText: '草稿正文', revision: 2 })
    const { client } = await setup({ saved: [saved] })
    await userEvent.click(screen.getByRole('button', { name: /^草稿/ }))
    await userEvent.click(await screen.findByText('草稿主题'))
    shiftClick(await screen.findByRole('button', { name: '继续编辑' }))
    await waitFor(() => expect(client.openComposer).toHaveBeenCalledWith(SAVED_ID, false))
    noComposer()
    await userEvent.click(screen.getByRole('button', { name: '继续编辑' }))
    expect(await within(await composer()).findByDisplayValue('草稿主题')).toBeInTheDocument()
  })
})

describe('sending and keys', () => {
  it('closes after the second press of 发送 and the reader shows what it did before', async () => {
    const { client } = await setup()
    client.queueSend.mockResolvedValue({} as never)
    await userEvent.click(rows()[0])
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    await userEvent.click(screen.getByRole('button', { name: '回复' }))
    const region = await composer()
    await within(region).findByDisplayValue('回复：季度计划')
    await userEvent.click(within(region).getByRole('button', { name: '发送' }))
    const confirm = await within(region).findByRole('button', { name: '确认发送' })
    // The confirm step ignores presses for a moment, so a double click cannot send.
    await act(() => new Promise((resolve) => setTimeout(resolve, 450)))
    await userEvent.click(confirm)
    await waitFor(() => expect(client.queueSend).toHaveBeenCalledWith(expect.objectContaining({ draftId: REPLY_ID })))
    await waitFor(noComposer)
    expect(await screen.findByRole('heading', { level: 2, name: '季度计划' })).toBeInTheDocument()
  })

  it('single-key shortcuts do nothing while focus is in the composer', async () => {
    const { client, user } = await setup()
    await user.click(rows()[0])
    await screen.findByRole('heading', { level: 2, name: '季度计划' })
    await user.click(screen.getByRole('button', { name: '转发' }))
    const region = await composer()
    // A button inside the composer is not a text field, and still owns its keys.
    act(() => within(region).getByRole('button', { name: '添加附件' }).focus())
    await user.keyboard('rae#sjk')
    expect(client.createDraftFrom).toHaveBeenCalledTimes(1)
    expect(client.archiveEmails).not.toHaveBeenCalled()
    expect(client.deleteEmails).not.toHaveBeenCalled()
    expect(client.setFlag).not.toHaveBeenCalled()
    expect(screen.getByRole('region', { name: '写邮件' })).toBeInTheDocument()
    act(() => within(region).getByRole('textbox', { name: '正文' }).focus())
    await user.keyboard('r/?')
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(client.createDraftFrom).toHaveBeenCalledTimes(1)
  })

  it('undoing a send reopens the draft inline when the setting says so', async () => {
    const saved = draft(SAVED_ID, { to: [{ name: null, address: 'li@example.org' }], subject: '草稿主题', revision: 2 })
    const { client } = await setup({ saved: [saved] })
    const listener = client.onOutbox.mock.calls.at(-1)![0]
    const now = Date.now()
    const queued: OutboxItem = { id: 'out-1', draftId: SAVED_ID, accountId: account.id, senderEmailAddress: account.emailAddress, messageId: '<m@x>', subject: '草稿主题', recipients: ['li@example.org'], recipientCount: 1,
      state: 'queued', notBefore: new Date(now + 10_000).toISOString(), dataStartedAt: null, attempts: 0, nextAttemptAt: null, firstQueuedAt: new Date(now).toISOString(),
      error: null, acceptedAt: null, serverReply: null, serverCopyMessageId: null, resolution: null, supersededBy: null, createdAt: new Date(now).toISOString(), updatedAt: new Date(now).toISOString() }
    client.getOutbox.mockResolvedValue(queued)
    client.cancelOutbox.mockResolvedValue({ ...queued, state: 'canceled' })
    await act(async () => listener({ accountId: account.id, outboxId: 'out-1', draftId: SAVED_ID, state: 'queued', attempts: 0, nextAttemptAt: null, errorReason: null }))
    await userEvent.click((await screen.findAllByRole('button', { name: '撤销', hidden: true }))[0])
    expect(await within(await composer()).findByDisplayValue('草稿主题')).toBeInTheDocument()
    expect(client.openComposer).not.toHaveBeenCalled()
  })
})
