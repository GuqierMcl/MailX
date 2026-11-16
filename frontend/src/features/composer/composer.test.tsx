import { act, cleanup, createEvent, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Toaster } from '@/components/ui/toast'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Draft } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { Composer } from './composer'
import { AUTOSAVE_DELAY_MS, SAVING_INDICATOR_DELAY_MS } from './use-draft-editor'


// The real editing engine is covered by mail-body-editor and composer-rich-text tests.
vi.mock('./mail-body-editor', () => ({
  default: ({ value, onChange, readOnly }: { value: string; onChange: (html: string) => void; readOnly?: boolean }) => (
    <textarea aria-label="正文" value={value} disabled={readOnly} onChange={(event) => onChange(event.target.value)} />
  ),
}))

const DRAFT_ID = '3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b'
const account = exampleAccount({ senderName: '林薇' })

function draft(overrides: Partial<Draft> = {}): Draft {
  return {
    id: DRAFT_ID, accountId: account.id, kind: 'new', origin: 'user', sourceEmailId: null, to: [], cc: [], bcc: [], subject: '', bodyText: overrides.bodyHtml?.replace(/<[^>]+>/g, '') ?? '', bodyHtml: '',
    inReplyTo: null, references: null, createdAt: '2026-10-23T00:00:00Z', updatedAt: '2026-10-23T00:00:00Z', revision: 1, attachments: [], ...overrides,
  }
}

function setup(initial: Draft = draft(), accounts = [account], extra: { headerActions?: React.ReactNode } = {}) {
  const client = testClient(accounts)
  let current = initial
  client.getDraft.mockImplementation(async () => structuredClone(current))
  client.updateDraft.mockImplementation(async (request) => {
    current = { ...current, to: request.to, cc: request.cc, bcc: request.bcc, subject: request.subject, bodyHtml: request.bodyHtml, bodyText: request.bodyHtml.replace(/<[^>]+>/g, ''), revision: current.revision + 1 }
    return structuredClone(current)
  })
  const onSent = vi.fn()
  const onDiscarded = vi.fn()
  const controller = { current: null as import('./composer').ComposerController | null }
  const view = render(
    <>
      <Toaster />
      <Composer client={client} draftId={DRAFT_ID} accounts={accounts} onSent={onSent} onDiscarded={onDiscarded} controllerRef={controller} {...extra} />
    </>,
  )
  return { client, onSent, onDiscarded, controller, view, setCurrent: (next: Draft) => { current = next } }
}

const body = () => screen.findByRole('textbox', { name: '正文' })
const status = () => document.querySelector('[data-status]') as HTMLElement
/** The expanded sender and recipient rows (the collapsed summary repeats some of their text). */
const headerRows = () => document.querySelector('[data-slot="composer-header-rows"]') as HTMLElement

afterEach(() => vi.useRealTimers())

describe('composer layout', () => {
  it('shows sender, recipients, subject, body and the bottom bar, with Cc and Bcc collapsed', async () => {
    setup()
    await body()
    const rows = headerRows()
    expect(within(rows).getByText('发件人')).toBeInTheDocument()
    // A single account is plain text: its name, then the address in small type.
    expect(within(rows).getByRole('group', { name: '发件人：工作邮箱' })).toHaveTextContent('工作邮箱work@example.com')
    expect(screen.queryByRole('combobox', { name: '发件人' })).not.toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: '收件人' })).toBeInTheDocument()
    expect(screen.queryByRole('combobox', { name: '抄送' })).not.toBeInTheDocument()
    expect(screen.queryByRole('combobox', { name: '密送' })).not.toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: '主题' })).toBeInTheDocument()
    for (const name of ['发送', '丢弃']) expect(screen.getByRole('button', { name })).toBeInTheDocument()
    expect(status()).toHaveTextContent(/^$/)
    await userEvent.click(screen.getByRole('button', { name: '抄送' }))
    expect(screen.getByRole('combobox', { name: '抄送' })).toBeInTheDocument()
  })

  it('labels the draft 由 MCP 创建 only when MCP created it', async () => {
    const first = setup()
    await body()
    expect(screen.queryByText('由 MCP 创建')).not.toBeInTheDocument()
    first.view.unmount()
    setup(draft({ origin: 'mcp' }))
    await body()
    expect(within(headerRows()).getByText('由 MCP 创建')).toBeInTheDocument()
    expect(screen.queryByText('由 AI 创建')).not.toBeInTheDocument()
  })

  it('labels the draft 由 AI 创建 only when the built-in AI created it', async () => {
    const first = setup()
    await body()
    expect(screen.queryByText('由 AI 创建')).not.toBeInTheDocument()
    first.view.unmount()
    setup(draft({ origin: 'ai' }))
    await body()
    expect(within(headerRows()).getByText('由 AI 创建')).toBeInTheDocument()
    expect(screen.queryByText('由 MCP 创建')).not.toBeInTheDocument()
  })

  it('opens Cc and Bcc rows when the draft already has those recipients', async () => {
    setup(draft({ cc: [{ name: null, address: 'c@x.com' }], bcc: [{ name: null, address: 'b@x.com' }] }))
    expect(await screen.findByRole('combobox', { name: '抄送' })).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: '密送' })).toBeInTheDocument()
  })
})

describe('autosave', () => {
  it('saves once after typing stops for a second, naming the last saved revision', async () => {
    const { client } = setup()
    const text = await body()
    vi.useFakeTimers()
    fireEvent.change(text, { target: { value: 'a' } })
    fireEvent.change(text, { target: { value: 'ab' } })
    expect(status()).toHaveTextContent(/^$/)
    await act(async () => { vi.advanceTimersByTime(AUTOSAVE_DELAY_MS - 10) })
    expect(client.updateDraft).not.toHaveBeenCalled()
    await act(async () => { vi.advanceTimersByTime(20) })
    expect(client.updateDraft).toHaveBeenCalledTimes(1)
    expect(client.updateDraft).toHaveBeenCalledWith(expect.objectContaining({ id: DRAFT_ID, expectedRevision: 1, bodyHtml: 'ab', accountId: account.id }))
    expect(status()).toHaveTextContent('已保存')
  })

  it('never has two saves in flight; edits made meanwhile go out afterwards at the new revision', async () => {
    const { client } = setup()
    const text = await body()
    let finish: (value: Draft) => void = () => undefined
    const first = new Promise<Draft>((resolve) => { finish = resolve })
    client.updateDraft.mockReset()
    client.updateDraft.mockReturnValueOnce(first).mockImplementation(async (request) => draft({ bodyHtml: request.bodyHtml, bodyText: request.bodyHtml.replace(/<[^>]+>/g, ''), revision: 3 }))
    vi.useFakeTimers()
    fireEvent.change(text, { target: { value: 'one' } })
    await act(async () => { vi.advanceTimersByTime(AUTOSAVE_DELAY_MS + 1) })
    expect(client.updateDraft).toHaveBeenCalledTimes(1)
    fireEvent.change(text, { target: { value: 'two' } })
    await act(async () => { vi.advanceTimersByTime(AUTOSAVE_DELAY_MS + 1) })
    expect(client.updateDraft).toHaveBeenCalledTimes(1)
    await act(async () => { finish(draft({ bodyHtml: 'one', revision: 2 })) })
    expect(client.updateDraft).toHaveBeenCalledTimes(2)
    expect(client.updateDraft).toHaveBeenLastCalledWith(expect.objectContaining({ expectedRevision: 2, bodyHtml: 'two' }))
  })

  it('reloads the latest draft and says so when the core reports a conflict', async () => {
    const { client, setCurrent } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValueOnce(new MailxClientError({ code: 'conflict', message: 'x', field: null }))
    setCurrent(draft({ bodyHtml: 'from elsewhere', revision: 5 }))
    fireEvent.change(text, { target: { value: 'mine' } })
    await waitFor(() => expect(client.updateDraft).toHaveBeenCalled(), { timeout: 3000 })
    await waitFor(() => expect(screen.getByRole('textbox', { name: '正文' })).toHaveValue('from elsewhere'))
    expect(await screen.findByText('草稿已在别处修改，已加载最新版本')).toBeInTheDocument()
    expect(status()).toHaveTextContent(/^$/)
  })

  it('turns read-only when the draft is locked by the send queue', async () => {
    const { client } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValueOnce(new MailxClientError({ code: 'draftLocked', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'late' } })
    await waitFor(() => expect(client.updateDraft).toHaveBeenCalled(), { timeout: 3000 })
    await waitFor(() => expect(screen.getByRole('textbox', { name: '正文' })).toBeDisabled())
    expect(screen.getByText(/已进入发送队列，不能再修改/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '发送' })).toBeDisabled()
  })

  it('follows the queue: a lock event makes it read-only, an unlock reloads it', async () => {
    const client = testClient([account])
    client.getDraft.mockResolvedValue(draft())
    let emit: (change: { draftId: string; accountId: string; change: 'locked' | 'unlocked' | 'deleted' }) => void = () => undefined
    client.onDrafts.mockImplementation((listener) => { emit = listener; return () => undefined })
    render(<Composer client={client} draftId={DRAFT_ID} accounts={[account]} onSent={vi.fn()} onDiscarded={vi.fn()} />)
    const text = await body()
    act(() => emit({ draftId: DRAFT_ID, accountId: account.id, change: 'locked' }))
    expect(text).toBeDisabled()
    act(() => emit({ draftId: DRAFT_ID, accountId: account.id, change: 'unlocked' }))
    await waitFor(() => expect(screen.getByRole('textbox', { name: '正文' })).toBeEnabled())
  })

  it('shows a failed save and lets the user retry', async () => {
    const { client } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValueOnce(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'x' } })
    await waitFor(() => expect(status()).toHaveTextContent('保存失败'), { timeout: 3000 })
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    await waitFor(() => expect(status()).toHaveTextContent('已保存'))
  })
})

describe('sending', () => {
  it('keeps confirmation when the core canonicalizes HTML during the preparation save', async () => {
    const { client } = setup(draft({ to: [{ name: null, address: 'li@example.org' }] }))
    const text = await body()
    client.updateDraft.mockResolvedValueOnce(draft({ to: [{ name: null, address: 'li@example.org' }], bodyHtml: '<p>&quot;quoted&quot;</p>', bodyText: '"quoted"', revision: 2 }))
    fireEvent.change(text, { target: { value: '<p>"quoted"</p>' } })
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await screen.findByRole('button', { name: '无主题，确认发送' })
    await waitFor(() => expect(text).toHaveValue('<p>&quot;quoted&quot;</p>'))
    expect(screen.getByRole('button', { name: '无主题，确认发送' })).toBeInTheDocument()
    expect(client.queueSend).not.toHaveBeenCalled()
  })

  async function fillAndPrepare(client: ReturnType<typeof testClient>) {
    await body()
    await userEvent.type(screen.getByRole('combobox', { name: '收件人' }), 'li@example.org{Enter}')
    await userEvent.type(screen.getByRole('textbox', { name: '主题' }), '你好')
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '确认发送' })).toBeInTheDocument())
    expect(client.updateDraft).toHaveBeenCalled()
  }

  it('refuses without a recipient or with an invalid one, and explains', async () => {
    const { client } = setup()
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('请至少填写一个收件人')
    await userEvent.type(screen.getByRole('combobox', { name: '收件人' }), 'nope{Enter}')
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(/地址无效/)
    expect(screen.getByRole('button', { name: '发送' })).toBeInTheDocument()
    expect(client.queueSend).not.toHaveBeenCalled()
  })

  it('saves first, then queues the saved revision with a fresh mutation id and closes', async () => {
    const { client, onSent } = setup()
    client.queueSend.mockResolvedValue({} as never)
    await fillAndPrepare(client)
    const saved = (await client.updateDraft.mock.results.at(-1)!.value as Draft).revision
    await new Promise((resolve) => setTimeout(resolve, 450))
    await userEvent.click(screen.getByRole('button', { name: '确认发送' }))
    await waitFor(() => expect(onSent).toHaveBeenCalled())
    expect(client.queueSend).toHaveBeenCalledWith({ draftId: DRAFT_ID, expectedRevision: saved, clientMutationId: expect.stringMatching(/^[0-9a-f-]{36}$/) })
  })

  it('reloads and reverts on a revision conflict instead of sending', async () => {
    const { client, onSent } = setup()
    client.queueSend.mockRejectedValue(new MailxClientError({ code: 'conflict', message: 'x', field: null }))
    await fillAndPrepare(client)
    await new Promise((resolve) => setTimeout(resolve, 450))
    await userEvent.click(screen.getByRole('button', { name: '确认发送' }))
    await waitFor(() => expect(client.getDraft.mock.calls.length).toBeGreaterThan(1))
    expect(onSent).not.toHaveBeenCalled()
    expect(await screen.findByRole('button', { name: '发送' })).toBeInTheDocument()
  })

  it('does not prepare when the account needs its sending credential confirmed', async () => {
    const client = testClient([exampleAccount({ authentication: { ...exampleAccount().authentication, smtp: { ...exampleAccount().authentication.smtp, trust: 'needsConfirmation' } } })])
    client.getDraft.mockResolvedValue(draft({ to: [{ name: null, address: 'a@b.co' }] }))
    render(<Composer client={client} draftId={DRAFT_ID} accounts={[exampleAccount({ authentication: { ...exampleAccount().authentication, smtp: { ...exampleAccount().authentication.smtp, trust: 'needsConfirmation' } } })]} onSent={vi.fn()} onDiscarded={vi.fn()} />)
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('凭证需要重新确认')
  })
})

describe('discard', () => {
  it('deletes an empty draft at once', async () => {
    const { client, onDiscarded } = setup()
    await body()
    await userEvent.click(screen.getByRole('button', { name: '丢弃' }))
    await waitFor(() => expect(onDiscarded).toHaveBeenCalled())
    expect(client.deleteDraft).toHaveBeenCalledWith(DRAFT_ID)
  })

  it('asks before discarding a draft with content', async () => {
    const { client, onDiscarded } = setup(draft({ subject: '重要' }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '丢弃' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(client.deleteDraft).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '继续编辑' }))
    await waitFor(() => expect(dialog).not.toBeInTheDocument())
    await userEvent.click(screen.getByRole('button', { name: '丢弃' }))
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '丢弃' }))
    await waitFor(() => expect(onDiscarded).toHaveBeenCalled())
    expect(client.deleteDraft).toHaveBeenCalledWith(DRAFT_ID)
  })
})

describe('save status', () => {
  it('shows nothing when a fresh or a reopened draft opens', async () => {
    setup()
    await body()
    expect(status()).toHaveTextContent(/^$/)
    expect(status()).toHaveAttribute('data-status', 'idle')
    expect(screen.queryByText('已保存')).not.toBeInTheDocument()
    cleanup()
    setup(draft({ subject: 'Hi', bodyHtml: 'text', revision: 4 }))
    await body()
    expect(status()).toHaveAttribute('data-status', 'idle')
    expect(screen.queryByText('已保存')).not.toBeInTheDocument()
  })

  it('keeps the live region polite', async () => {
    setup()
    await body()
    expect(status()).toHaveAttribute('role', 'status')
    expect(status()).toHaveAttribute('aria-live', 'polite')
  })

  it('goes straight to 已保存 when the save is fast, never showing 正在保存…', async () => {
    setup()
    const text = await body()
    vi.useFakeTimers()
    fireEvent.change(text, { target: { value: 'hello' } })
    await act(async () => { vi.advanceTimersByTime(AUTOSAVE_DELAY_MS + 1) })
    expect(status()).toHaveTextContent('已保存')
    expect(screen.queryByText('正在保存…')).not.toBeInTheDocument()
  })

  it('shows 正在保存… only once a save has taken longer than the delay', async () => {
    const { client } = setup()
    const text = await body()
    let finish: (value: Draft) => void = () => undefined
    client.updateDraft.mockReset()
    client.updateDraft.mockReturnValueOnce(new Promise<Draft>((resolve) => { finish = resolve }))
    vi.useFakeTimers()
    fireEvent.change(text, { target: { value: 'slow' } })
    await act(async () => { vi.advanceTimersByTime(AUTOSAVE_DELAY_MS + 1) })
    expect(client.updateDraft).toHaveBeenCalledTimes(1)
    await act(async () => { vi.advanceTimersByTime(SAVING_INDICATOR_DELAY_MS - 50) })
    expect(status()).toHaveTextContent(/^$/)
    await act(async () => { vi.advanceTimersByTime(100) })
    expect(status()).toHaveTextContent('正在保存…')
    await act(async () => { finish(draft({ bodyHtml: 'slow', revision: 2 })) })
    expect(status()).toHaveTextContent('已保存')
  })

  it('goes back to showing nothing when the content is cleared to blank', async () => {
    setup()
    const text = await body()
    vi.useFakeTimers()
    fireEvent.change(text, { target: { value: 'x' } })
    await act(async () => { vi.advanceTimersByTime(AUTOSAVE_DELAY_MS + 1) })
    expect(status()).toHaveTextContent('已保存')
    fireEvent.change(text, { target: { value: '' } })
    await act(async () => { vi.advanceTimersByTime(AUTOSAVE_DELAY_MS + 1) })
    expect(status()).toHaveAttribute('data-status', 'idle')
    expect(status()).toHaveTextContent(/^$/)
  })

  it('says the draft is in the send queue instead of a save status when read-only', async () => {
    const client = testClient([account])
    client.getDraft.mockResolvedValue(draft({ bodyHtml: 'x' }))
    let emit: (change: { draftId: string; accountId: string; change: 'locked' | 'unlocked' | 'deleted' }) => void = () => undefined
    client.onDrafts.mockImplementation((listener) => { emit = listener; return () => undefined })
    render(<Composer client={client} draftId={DRAFT_ID} accounts={[account]} onSent={vi.fn()} onDiscarded={vi.fn()} />)
    await body()
    act(() => emit({ draftId: DRAFT_ID, accountId: account.id, change: 'locked' }))
    expect(status()).toHaveTextContent('已在发送队列中')
    expect(screen.queryByRole('button', { name: '重试' })).not.toBeInTheDocument()
  })

  it('shows 保存失败 with a 重试 button that saves again', async () => {
    const { client } = setup()
    const text = await body()
    client.updateDraft.mockRejectedValueOnce(new MailxClientError({ code: 'storageUnavailable', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'x' } })
    await waitFor(() => expect(status()).toHaveTextContent('保存失败'), { timeout: 3000 })
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    await waitFor(() => expect(status()).toHaveTextContent('已保存'))
    expect(screen.queryByRole('button', { name: '重试' })).not.toBeInTheDocument()
  })
})

describe('attachments', () => {
  const file = { id: 'a1', fileName: '报表.pdf', mediaType: 'application/pdf', size: 2048, origin: 'user' as const, inline: false, contentId: null }

  it('attaches through the host picker at the saved revision and lists the file', async () => {
    const { client } = setup()
    await body()
    client.addAttachments.mockResolvedValue(draft({ revision: 2, attachments: [file] }))
    await userEvent.click(screen.getByRole('button', { name: '添加附件' }))
    expect(client.addAttachments).toHaveBeenCalledWith(DRAFT_ID, 1)
    const list = await screen.findByRole('list', { name: '附件' })
    expect(within(list).getByText('报表.pdf')).toBeInTheDocument()
    expect(within(list).getByText('2 KB')).toBeInTheDocument()
  })

  it('writes pending text first and keeps it: only the revision advances after an attachment command', async () => {
    const { client } = setup()
    const text = await body()
    fireEvent.change(text, { target: { value: '<p>新内容</p>' } })
    client.addAttachments.mockResolvedValue(draft({ revision: 3, attachments: [file] }))
    await userEvent.click(screen.getByRole('button', { name: '添加附件' }))
    expect(client.updateDraft).toHaveBeenCalled()
    expect(client.addAttachments).toHaveBeenCalledWith(DRAFT_ID, 2)
    expect(await body()).toHaveValue('<p>新内容</p>')
    fireEvent.change(text, { target: { value: '<p>再改</p>' } })
    await waitFor(() => expect(client.updateDraft).toHaveBeenCalledTimes(2), { timeout: 3000 })
    expect(client.updateDraft.mock.calls[1][0].expectedRevision).toBe(3)
  })

  it('removes an attachment by id', async () => {
    const { client } = setup(draft({ attachments: [file] }))
    await body()
    client.removeAttachment.mockResolvedValue(draft({ revision: 2 }))
    await userEvent.click(await screen.findByRole('button', { name: '移除 报表.pdf' }))
    expect(client.removeAttachment).toHaveBeenCalledWith({ draftId: DRAFT_ID, attachmentId: 'a1', expectedRevision: 1 })
    await waitFor(() => expect(screen.queryByText('报表.pdf')).not.toBeInTheDocument())
  })

  it('attaches the dropped files when the host reports a drop', async () => {
    const { client } = setup()
    await body()
    const listener = client.onFilesDropped.mock.calls[0][0]
    client.addDroppedAttachments.mockResolvedValue(draft({ revision: 2, attachments: [file] }))
    act(() => listener())
    await waitFor(() => expect(client.addDroppedAttachments).toHaveBeenCalledWith(DRAFT_ID, 1))
    expect(await screen.findByText('报表.pdf')).toBeInTheDocument()
  })

  it('shows a typed core error and resyncs the list after a failed add', async () => {
    const { client } = setup()
    await body()
    client.addAttachments.mockRejectedValue(new MailxClientError({ code: 'attachmentTooLarge', message: 'x', field: null }))
    client.getDraft.mockResolvedValue(draft({ revision: 2, attachments: [file] }))
    await userEvent.click(screen.getByRole('button', { name: '添加附件' }))
    expect(await screen.findByText('附件总大小超过上限。')).toBeInTheDocument()
    expect(await screen.findByText('报表.pdf')).toBeInTheDocument()
  })

  it('shows attachments of a locked draft but offers no change', async () => {
    const { client } = setup(draft({ attachments: [file] }))
    const text = await body()
    client.updateDraft.mockRejectedValueOnce(new MailxClientError({ code: 'draftLocked', message: 'x', field: null }))
    fireEvent.change(text, { target: { value: 'late' } })
    await waitFor(() => expect(screen.getByRole('textbox', { name: '正文' })).toBeDisabled(), { timeout: 3000 })
    expect(screen.getByText('报表.pdf')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '移除 报表.pdf' })).toBeDisabled()
    expect(screen.getByRole('button', { name: '添加附件' })).toBeDisabled()
  })

  it('marks an over-limit draft and still lets the core refuse the send', async () => {
    const big = { ...file, size: 25 * 1024 * 1024 }
    const { client } = setup(draft({ attachments: [big] }))
    client.attachmentPolicy.mockResolvedValue({ maxTotalBytes: 20 * 1024 * 1024, maxCount: 20, serverLimitMb: null })
    await body()
    expect(await screen.findByText(/附件总大小超过 20\.0 MB 的上限/)).toBeInTheDocument()
    client.queueSend.mockRejectedValue(new MailxClientError({ code: 'attachmentTooLarge', message: 'x', field: null }))
    await userEvent.type(screen.getByRole('combobox', { name: '收件人' }), 'li@example.org{Enter}')
    await userEvent.type(screen.getByRole('textbox', { name: '主题' }), '你好')
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await screen.findByRole('button', { name: '确认发送' })
    await new Promise((resolve) => setTimeout(resolve, 450))
    await userEvent.click(screen.getByRole('button', { name: '确认发送' }))
    expect(await screen.findByText('附件总大小超过上限。')).toBeInTheDocument()
  })

  it('hints at a smaller server limit without blocking', async () => {
    const { client } = setup(draft({ attachments: [{ ...file, size: 3 * 1024 * 1024 }] }))
    client.attachmentPolicy.mockResolvedValue({ maxTotalBytes: 20 * 1024 * 1024, maxCount: 20, serverLimitMb: 2 })
    await body()
    expect(await screen.findByText(/约 2 MB/)).toBeInTheDocument()
  })
})

const zhang = { name: '张三', address: 'zs@example.com' }
const li = { name: null, address: 'li@example.org' }
const wang = { name: '王五', address: 'ww@example.com' }
const rowsCollapsed = () => headerRows().hasAttribute('data-collapsed')
const summary = () => document.querySelector('[data-slot="composer-header-summary"]') as HTMLElement

describe('sender picker', () => {
  const personal = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人邮箱', emailAddress: 'me@home.example', color: '#16a34a' })

  it('names accounts by account name, the address after it, and lists what recipients will see', async () => {
    setup(draft(), [account, personal])
    await body()
    const picker = screen.getByRole('combobox', { name: '发件人' })
    expect(picker).toHaveTextContent('工作邮箱work@example.com')
    await userEvent.click(picker)
    const options = await screen.findAllByRole('option')
    // Each option: avatar, account name, then the sender line (the avatar's initial is decorative text).
    expect(options[0]).toHaveTextContent(/工作邮箱\s*林薇 <work@example.com>$/)
    expect(options[1]).toHaveTextContent(/个人邮箱\s*me@home.example$/)
    expect(options[0]).toHaveAttribute('aria-selected', 'true')
  })

  it('is plain text with one account, read as 发件人：<name>', async () => {
    setup()
    await body()
    expect(within(headerRows()).getByRole('group', { name: '发件人：工作邮箱' })).toBeInTheDocument()
    expect(screen.queryByRole('combobox', { name: '发件人' })).not.toBeInTheDocument()
  })

  it('puts the MCP note and the host controls at the end of the From row', async () => {
    setup(draft({ origin: 'mcp' }), [account], { headerActions: <button type="button">关闭写信</button> })
    await body()
    const fromRow = document.getElementById('composer-from-label')!.parentElement!
    expect(within(fromRow).getByText('由 MCP 创建')).toBeInTheDocument()
    const actions = document.querySelector('[data-slot="composer-header-actions"]') as HTMLElement
    expect(within(actions).getByRole('button', { name: '关闭写信' })).toBeInTheDocument()
  })
})

describe('Cc and Bcc rows', () => {
  it('open and take focus with Cmd/Ctrl+Alt+C and +B, matched by physical key', async () => {
    setup()
    await body()
    const subject = screen.getByRole('textbox', { name: '主题' })
    // macOS Option changes the character (ç), so only `code` identifies the key.
    fireEvent.keyDown(subject, { key: 'ç', code: 'KeyC', metaKey: true, altKey: true })
    expect(await screen.findByRole('combobox', { name: '抄送' })).toHaveFocus()
    fireEvent.keyDown(screen.getByRole('combobox', { name: '抄送' }), { key: 'b', code: 'KeyB', ctrlKey: true, altKey: true })
    expect(await screen.findByRole('combobox', { name: '密送' })).toHaveFocus()
    // Already open: the shortcut only moves focus.
    fireEvent.keyDown(screen.getByRole('combobox', { name: '密送' }), { key: 'c', code: 'KeyC', ctrlKey: true, altKey: true })
    await waitFor(() => expect(screen.getByRole('combobox', { name: '抄送' })).toHaveFocus())
  })

  it('leave AltGr (Ctrl+Alt on Windows layouts) typing alone', async () => {
    setup()
    await body()
    const subject = screen.getByRole('textbox', { name: '主题' })
    const event = createEvent.keyDown(subject, { key: 'ć', code: 'KeyC', ctrlKey: true, altKey: true, modifierAltGraph: true })
    fireEvent(subject, event)
    expect(event.defaultPrevented).toBe(false)
    expect(screen.queryByRole('combobox', { name: '抄送' })).not.toBeInTheDocument()
  })

  it('reach the shortcut before the body editor does', async () => {
    setup()
    const text = await body()
    const event = createEvent.keyDown(text, { key: 'ç', code: 'KeyC', metaKey: true, altKey: true })
    const inner = vi.fn()
    text.addEventListener('keydown', inner)
    fireEvent(text, event)
    expect(inner).not.toHaveBeenCalled()
    expect(event.defaultPrevented).toBe(true)
    expect(await screen.findByRole('combobox', { name: '抄送' })).toHaveFocus()
  })

  it('show the shortcut in the toggle tooltips', async () => {
    setup()
    await body()
    await userEvent.hover(screen.getByRole('button', { name: '密送' }))
    expect(await screen.findByText('密送（Cmd/Ctrl+Alt+B）')).toBeInTheDocument()
  })

  it('fold away when left empty, and stay when they have an address', async () => {
    setup()
    await body()
    await userEvent.click(screen.getByRole('button', { name: '抄送' }))
    expect(screen.getByRole('combobox', { name: '抄送' })).toHaveFocus()
    await userEvent.click(screen.getByRole('textbox', { name: '主题' }))
    expect(screen.queryByRole('combobox', { name: '抄送' })).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: '抄送' }))
    await userEvent.type(screen.getByRole('combobox', { name: '抄送' }), 'c@x.com')
    await userEvent.click(screen.getByRole('textbox', { name: '主题' }))
    expect(screen.getByRole('combobox', { name: '抄送' })).toBeInTheDocument()
  })

  it('keep a draft’s Cc row open while its last address is removed, then fold once left', async () => {
    setup(draft({ cc: [li] }))
    const cc = await screen.findByRole('combobox', { name: '抄送' })
    await userEvent.click(cc)
    await userEvent.keyboard('{Backspace}')
    expect(cc).toBeInTheDocument()
    await userEvent.click(screen.getByRole('textbox', { name: '主题' }))
    expect(screen.queryByRole('combobox', { name: '抄送' })).not.toBeInTheDocument()
  })
})

describe('moving recipients', () => {
  it('moves a chip to another row from its menu, keeping one copy and opening the row', async () => {
    const { client } = setup(draft({ to: [zhang, li], bcc: [li] }))
    await body()
    fireEvent.contextMenu(within(headerRows()).getByRole('button', { name: '张三' }))
    await userEvent.click(await screen.findByRole('menuitem', { name: '移到抄送' }))
    const cc = screen.getByRole('combobox', { name: '抄送' }).closest('[data-slot="recipient-field"]') as HTMLElement
    expect(within(cc).getByRole('button', { name: '张三' })).toBeInTheDocument()

    fireEvent.contextMenu(within(headerRows()).getAllByRole('button', { name: 'li@example.org' })[0])
    await userEvent.click(await screen.findByRole('menuitem', { name: '移到密送' }))
    const bcc = screen.getByRole('combobox', { name: '密送' }).closest('[data-slot="recipient-field"]') as HTMLElement
    expect(within(bcc).getAllByRole('button', { name: 'li@example.org' })).toHaveLength(1)

    // The Cc row was emptied from elsewhere, so it folds away.
    fireEvent.contextMenu(within(cc).getByRole('button', { name: '张三' }))
    await userEvent.click(await screen.findByRole('menuitem', { name: '移到收件人' }))
    expect(screen.queryByRole('combobox', { name: '抄送' })).not.toBeInTheDocument()
    await waitFor(() => expect(client.updateDraft).toHaveBeenLastCalledWith(expect.objectContaining({ to: [zhang], cc: [], bcc: [li] })), { timeout: 3000 })
  })
})

describe('header collapse', () => {
  it('folds sender and recipients into one line while the body has keyboard focus, keeping the subject', async () => {
    setup(draft({ to: [zhang, li], cc: [wang], subject: '周报' }))
    const text = await body()
    expect(rowsCollapsed()).toBe(false)
    act(() => text.focus())
    expect(rowsCollapsed()).toBe(true)
    // Nothing the user did caused it (a reply opens with the body focused), so it does not animate.
    expect(headerRows().className).not.toContain('transition')
    expect(headerRows().firstElementChild).toHaveAttribute('inert')
    expect(summary()).toHaveAccessibleName('展开发件人和收件人：工作邮箱 → 张三、li@example.org 等 3 人')
    expect(summary().parentElement).not.toHaveAttribute('inert')
    expect(screen.getByRole('textbox', { name: '主题' })).toBeVisible()
    // The subject is part of the header that stays; focusing it keeps the summary.
    act(() => screen.getByRole('textbox', { name: '主题' }).focus())
    expect(rowsCollapsed()).toBe(true)
  })

  it('waits for a click to finish before folding, so the text under the pointer stays put', async () => {
    setup(draft({ to: [zhang] }))
    const text = await body()
    await userEvent.click(text)
    expect(rowsCollapsed()).toBe(false)
    await waitFor(() => expect(rowsCollapsed()).toBe(true), { timeout: 2000 })
    expect(headerRows().className).toContain('duration-150')
  })

  it('folds at once when typing starts after a click', async () => {
    setup(draft({ to: [zhang] }))
    const text = await body()
    await userEvent.click(text)
    await userEvent.keyboard('a')
    expect(rowsCollapsed()).toBe(true)
  })

  it('expands from the summary by click or Enter and puts focus in the To field', async () => {
    setup(draft({ to: [zhang] }))
    const text = await body()
    act(() => text.focus())
    await userEvent.click(summary())
    expect(rowsCollapsed()).toBe(false)
    expect(screen.getByRole('combobox', { name: '收件人' })).toHaveFocus()

    act(() => text.focus())
    act(() => summary().focus())
    await userEvent.keyboard('{Enter}')
    expect(rowsCollapsed()).toBe(false)
    expect(screen.getByRole('combobox', { name: '收件人' })).toHaveFocus()
  })

  it('expands for the Cc shortcut', async () => {
    setup(draft({ to: [zhang] }))
    const text = await body()
    act(() => text.focus())
    fireEvent.keyDown(text, { key: 'ç', code: 'KeyC', metaKey: true, altKey: true })
    expect(rowsCollapsed()).toBe(false)
    expect(await screen.findByRole('combobox', { name: '抄送' })).toHaveFocus()
  })

  it('stays expanded without recipients, with an invalid address or with an error showing', async () => {
    const first = setup()
    let text = await body()
    act(() => text.focus())
    expect(rowsCollapsed()).toBe(false)
    first.view.unmount()

    const second = setup()
    text = await body()
    await userEvent.type(screen.getByRole('combobox', { name: '收件人' }), 'nope,')
    act(() => text.focus())
    expect(rowsCollapsed()).toBe(false)
    second.view.unmount()

    setup(draft({ to: [zhang] }), [exampleAccount({ id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc' })])
    text = await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(await screen.findByRole('alert')).toBeInTheDocument()
    act(() => text.focus())
    expect(rowsCollapsed()).toBe(false)
  })
})

describe('forgotten attachment reminder', () => {
  const file = { id: 'a1', fileName: '报表.pdf', mediaType: 'application/pdf', size: 2048, origin: 'user' as const, inline: false, contentId: null }
  const reminder = () => document.querySelector('[data-slot="attachment-reminder"]') as HTMLElement | null
  const sendButton = () => document.querySelector('[data-stage]') as HTMLElement

  it('stops the first press when the body mentions an attachment and none is added', async () => {
    const { client } = setup(draft({ to: [zhang], subject: '报价', bodyHtml: '<p>报价单见附件。</p>' }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(await screen.findByText('正文提到了附件，但还没有添加')).toBeInTheDocument()
    expect(sendButton()).toHaveAttribute('data-stage', 'idle')
    expect(client.queueSend).not.toHaveBeenCalled()
  })

  it('checks a new message’s subject too, in either language, and ignores inline images', async () => {
    setup(draft({ to: [zhang], subject: 'Contract attached', attachments: [{ ...file, inline: true, contentId: 'c1' }] }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(await screen.findByText('正文提到了附件，但还没有添加')).toBeInTheDocument()
  })

  it('does not ask when a file is attached, or when only the quoted original mentions one', async () => {
    const first = setup(draft({ to: [zhang], bodyHtml: '<p>见附件</p>', attachments: [file] }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sendButton()).toHaveAttribute('data-stage', 'confirm'))
    expect(reminder()).toBeNull()
    first.view.unmount()

    setup(draft({ kind: 'reply', to: [zhang], subject: 'Re: 报价', bodyHtml: '<p>收到，谢谢</p><p>张三 写道：</p><blockquote><p>报价单见附件</p></blockquote>' }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sendButton()).toHaveAttribute('data-stage', 'confirm'))
    expect(reminder()).toBeNull()
  })

  it('checks a forward above its header only, and never an inherited subject', async () => {
    const original = '<p></p><p>---------- 转发的邮件 ----------</p><p>发件人：张三</p><p>主题：合同附件</p><p>合同见附件</p>'
    const first = setup(draft({ kind: 'forward', to: [zhang], subject: 'Fwd: 合同附件', bodyHtml: `<p>请看</p>${original}` }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sendButton()).toHaveAttribute('data-stage', 'confirm'))
    expect(reminder()).toBeNull()
    first.view.unmount()

    setup(draft({ kind: 'forward', to: [zhang], subject: 'Fwd: 合同', bodyHtml: `<p>附上我的批注</p>${original}` }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(await screen.findByText('正文提到了附件，但还没有添加')).toBeInTheDocument()
  })

  it('sits in its own row above the bottom bar, leaving the save status there', async () => {
    setup(draft({ to: [zhang], bodyHtml: '<p>附上合同</p>' }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(reminder()!.nextElementSibling).toContainElement(sendButton())
    expect(reminder()).toHaveTextContent('正文提到了附件，但还没有添加添加附件仍要发送')
    expect(status()).toBeVisible()
  })

  it('仍要发送 goes straight to the confirm stage, which still needs the second press', async () => {
    const { client } = setup(draft({ to: [zhang], bodyHtml: '<p>Please see the enclosed form.</p>' }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    await userEvent.click(within(reminder()!).getByRole('button', { name: '仍要发送' }))
    await waitFor(() => expect(sendButton()).toHaveAttribute('data-stage', 'confirm'))
    expect(sendButton()).toHaveFocus()
    expect(reminder()).toBeNull()
    expect(client.queueSend).not.toHaveBeenCalled()
    await new Promise((resolve) => setTimeout(resolve, 450))
    await userEvent.click(sendButton())
    expect(client.queueSend).toHaveBeenCalledTimes(1)
  })

  it('添加附件 opens the picker; once a file is added the notice goes and the button stays at 发送', async () => {
    const { client } = setup(draft({ to: [zhang], bodyHtml: '<p>附上合同</p>' }))
    await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    client.addAttachments.mockResolvedValue(draft({ revision: 2, to: [zhang], bodyHtml: '<p>附上合同</p>', attachments: [file] }))
    await userEvent.click(within(reminder()!).getByRole('button', { name: '添加附件' }))
    expect(client.addAttachments).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(reminder()).toBeNull())
    expect(sendButton()).toHaveAttribute('data-stage', 'idle')
  })

  it('goes away on any edit and is checked again on the next press', async () => {
    setup(draft({ to: [zhang], bodyHtml: '<p>附上合同</p>' }))
    const text = await body()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(reminder()).not.toBeNull()
    fireEvent.change(text, { target: { value: '<p>附上合同。</p>' } })
    expect(reminder()).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(reminder()).not.toBeNull()
  })
})

describe('recipient suggestions in the composer', () => {
  it('asks the client, leaves out addresses in any row and keeps the header open while picking', async () => {
    const { client } = setup(draft({ cc: [li] }))
    await body()
    client.suggestContacts.mockResolvedValue([{ name: null, address: 'li@example.org' }, { name: 'Lily', address: 'lily@example.com' }])
    await userEvent.type(screen.getByRole('combobox', { name: '收件人' }), 'li')
    const list = await screen.findByRole('listbox')
    expect(client.suggestContacts).toHaveBeenCalledWith('li', expect.any(Number))
    expect(within(list).getAllByRole('option').map((option) => option.textContent)).toEqual(['Lilylily@example.com'])
    await userEvent.click(within(list).getByRole('option'))
    const to = screen.getByRole('combobox', { name: '收件人' }).closest('[data-slot="recipient-field"]') as HTMLElement
    expect(within(to).getByRole('button', { name: 'Lily' })).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: '收件人' })).toHaveFocus()
    expect(rowsCollapsed()).toBe(false)
  })
})
