import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@/test/render'
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
    id: DRAFT_ID, accountId: account.id, kind: 'new', sourceEmailId: null, to: [], cc: [], bcc: [], subject: '', bodyText: overrides.bodyHtml?.replace(/<[^>]+>/g, '') ?? '', bodyHtml: '',
    inReplyTo: null, references: null, createdAt: '2026-10-23T00:00:00Z', updatedAt: '2026-10-23T00:00:00Z', revision: 1, attachments: [], ...overrides,
  }
}

function setup(initial: Draft = draft()) {
  const client = testClient([account])
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
      <Composer client={client} draftId={DRAFT_ID} accounts={[account]} onSent={onSent} onDiscarded={onDiscarded} controllerRef={controller} />
    </>,
  )
  return { client, onSent, onDiscarded, controller, view, setCurrent: (next: Draft) => { current = next } }
}

const body = () => screen.findByRole('textbox', { name: '正文' })
const status = () => document.querySelector('[data-status]') as HTMLElement

afterEach(() => vi.useRealTimers())

describe('composer layout', () => {
  it('shows sender, recipients, subject, body and the bottom bar, with Cc and Bcc collapsed', async () => {
    setup()
    await body()
    expect(screen.getByText('发件人')).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: '发件人' })).toHaveTextContent('林薇 <work@example.com>')
    expect(screen.getByRole('textbox', { name: '收件人' })).toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: '抄送' })).not.toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: '密送' })).not.toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: '主题' })).toBeInTheDocument()
    for (const name of ['发送', '丢弃']) expect(screen.getByRole('button', { name })).toBeInTheDocument()
    expect(status()).toHaveTextContent(/^$/)
    await userEvent.click(screen.getByRole('button', { name: '抄送' }))
    expect(screen.getByRole('textbox', { name: '抄送' })).toBeInTheDocument()
  })

  it('opens Cc and Bcc rows when the draft already has those recipients', async () => {
    setup(draft({ cc: [{ name: null, address: 'c@x.com' }], bcc: [{ name: null, address: 'b@x.com' }] }))
    expect(await screen.findByRole('textbox', { name: '抄送' })).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: '密送' })).toBeInTheDocument()
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
    await userEvent.type(screen.getByRole('textbox', { name: '收件人' }), 'li@example.org{Enter}')
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
    await userEvent.type(screen.getByRole('textbox', { name: '收件人' }), 'nope{Enter}')
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
