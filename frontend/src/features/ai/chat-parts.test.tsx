import { afterEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor, within } from '@/test/render'
import { TooltipProvider } from '@/components/ui/tooltip'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Draft, EmailDetail, EmailSummary, ToolActivity } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { i18n } from '@/i18n'
import { ChatEnvContext, type ChatEnv } from './chat-env'
import { ChatMessages } from './chat-messages'
import type { ChatMessage, ChatTurn } from './chat-model'
import type { RunTool } from './use-assistant-run'
import { toolLabel, UNDO_WINDOW_MS } from './tool-labels'
import { ToolRow } from './tool-parts'

const account = exampleAccount({ displayName: '工作邮箱' })
const summary = (id: string, over: Partial<EmailSummary> = {}): EmailSummary => ({
  senderGroup: null, id, accountId: account.id, folderId: 'inbox', subject: `邮件 ${id}`, from: { name: '王五', address: 'wang@example.com' }, preview: '',
  receivedAt: '2026-11-16T08:00:00Z', isRead: false, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null,
  to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...over,
})
const detail = (id: string): EmailDetail => ({ summary: summary(id), to: [], cc: [], replyTo: [], body: null, bodyFailure: null, attachments: [] })

const tool = (over: Partial<ToolActivity> & { settledAt?: number | null; at?: number }): RunTool => ({
  callId: 'k1', name: 'mail_archive', status: 'done', summary: { type: 'done' }, at: 0, settledAt: Date.now(), ...over,
})

function setup(env: Partial<ChatEnv> = {}) {
  const client = testClient([account])
  const handlers = { onOpenEmail: vi.fn(), onOpenDraft: vi.fn(), respond: vi.fn(), retry: vi.fn(), focusInput: vi.fn(), markUndone: vi.fn(), regenerate: vi.fn(), continueReply: vi.fn(), onEdit: vi.fn() }
  const value: ChatEnv = { client, accounts: [], folders: [], onOpenFolder: vi.fn(), undone: new Set(), ...handlers, ...env }
  const view = (children: React.ReactNode) => <TooltipProvider><ChatEnvContext value={value}>{children}</ChatEnvContext></TooltipProvider>
  return { client, ...handlers, view }
}

afterEach(() => { vi.useRealTimers() })

describe('ToolRow', () => {
  it.each([
    ['mail_conversation', '正在查看往来邮件…', '找到了 3 封往来邮件'],
    ['mail_contacts', '正在查找联系人…', '找到了 3 位联系人'],
  ])('describes %s with its found count', (name, running, done) => {
    const { view } = setup()
    const { rerender } = render(view(<ToolRow tool={tool({ name, status: 'running', summary: { type: 'pending' } })} />))
    expect(screen.getByText(running)).toBeInTheDocument()
    rerender(view(<ToolRow tool={tool({ name, summary: { type: 'found', count: 3 } })} />))
    expect(screen.getByText(done)).toBeInTheDocument()
    expect(screen.queryByText('完成了一步')).not.toBeInTheDocument()
  })

  it.each([
    ['mail_conversation', 'Found 1 email in the conversation', 'Found 2 emails in the conversation'],
    ['mail_contacts', 'Found 1 contact', 'Found 2 contacts'],
  ])('uses English singular and plural counts for %s', async (name, singular, plural) => {
    await i18n.changeLanguage('en')
    try {
      expect(toolLabel(tool({ name, summary: { type: 'found', count: 1 } }))).toBe(singular)
      expect(toolLabel(tool({ name, summary: { type: 'found', count: 2 } }))).toBe(plural)
    } finally {
      await i18n.changeLanguage('zh-CN')
    }
  })

  it('says what it is doing, then what it did, without raw tool names', () => {
    const { view } = setup()
    const { rerender } = render(view(<ToolRow tool={tool({ name: 'mail_search', status: 'running', summary: { type: 'pending' } })} />))
    expect(screen.getByText('正在搜索邮件…')).toBeInTheDocument()
    rerender(view(<ToolRow tool={tool({ name: 'mail_search', summary: { type: 'found', count: 12 } })} />))
    expect(screen.getByText('搜索了 12 封邮件')).toBeInTheDocument()
    expect(screen.queryByText(/mail_search/)).not.toBeInTheDocument()
    rerender(view(<ToolRow tool={tool({ name: 'mystery_tool', summary: { type: 'done' } })} />))
    expect(screen.getByText('完成了一步')).toBeInTheDocument()
    rerender(view(<ToolRow tool={tool({ name: 'mail_search', status: 'failed', summary: { type: 'error', code: 'x' } })} />))
    expect(screen.getByText('这一步没有成功')).toBeInTheDocument()
  })

  it('expands a read step to the emails it touched and opens one', async () => {
    const { client, view, onOpenEmail } = setup()
    client.getEmail.mockImplementation(async (id) => detail(id))
    render(view(<ToolRow tool={tool({ name: 'mail_read', summary: { type: 'read', emailIds: ['m1'] } })} />))
    expect(screen.getByText('阅读了 1 封邮件')).toBeInTheDocument()
    expect(client.getEmail).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '展开详情' }))
    const chip = await screen.findByRole('button', { name: /王五 · 邮件 m1/ })
    await userEvent.click(chip)
    expect(onOpenEmail).toHaveBeenCalledWith({ id: 'm1', accountId: account.id })
  })

  it('undoes an archive with the token from the step, once', async () => {
    const { client, view, markUndone } = setup()
    render(view(<ToolRow tool={tool({ summary: { type: 'changed', count: 3, emailIds: [], undo: { type: 'archive', token: 'tok-1' } } })} />))
    expect(screen.getByText('归档了 3 封')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '撤销' }))
    expect(client.undoArchive).toHaveBeenCalledWith('tok-1')
    await waitFor(() => expect(markUndone).toHaveBeenCalledWith('k1'))
  })

  it('writes flags back to their old values, grouped', async () => {
    const { client, view } = setup()
    render(view(<ToolRow tool={tool({
      name: 'mail_flags',
      summary: {
        type: 'changed', count: 3, emailIds: [],
        undo: { type: 'flags', restore: [
          { emailId: 'a', isRead: false, isStarred: null }, { emailId: 'b', isRead: false, isStarred: true }, { emailId: 'c', isRead: true, isStarred: null },
        ] },
      },
    })} />))
    await userEvent.click(screen.getByRole('button', { name: '撤销' }))
    await waitFor(() => expect(client.setFlags).toHaveBeenCalledTimes(3))
    expect(client.setFlags).toHaveBeenCalledWith(['a', 'b'], 'read', false)
    expect(client.setFlags).toHaveBeenCalledWith(['c'], 'read', true)
    expect(client.setFlags).toHaveBeenCalledWith(['b'], 'starred', true)
  })

  it('shows 已撤销 after the undo, and drops the button when the host no longer has it', async () => {
    const { client, view } = setup({ undone: new Set(['k1']) })
    const { rerender } = render(view(<ToolRow tool={tool({ summary: { type: 'changed', count: 1, emailIds: [], undo: { type: 'archive', token: 't' } } })} />))
    expect(screen.getByText('已撤销')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '撤销' })).not.toBeInTheDocument()
    const fresh = setup()
    fresh.client.undoArchive.mockRejectedValue(new MailxClientError({ code: 'notFound', message: '', field: null }))
    rerender(fresh.view(<ToolRow tool={tool({ callId: 'k2', summary: { type: 'changed', count: 1, emailIds: [], undo: { type: 'archive', token: 't' } } })} />))
    await userEvent.click(screen.getByRole('button', { name: '撤销' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: '撤销' })).not.toBeInTheDocument())
    expect(client.undoArchive).not.toHaveBeenCalled()
  })

  it('takes the undo button away after a minute', () => {
    vi.useFakeTimers()
    const { view } = setup()
    render(view(<ToolRow tool={tool({ settledAt: Date.now(), summary: { type: 'changed', count: 1, emailIds: [], undo: { type: 'archive', token: 't' } } })} />))
    expect(screen.getByRole('button', { name: '撤销' })).toBeInTheDocument()
    act(() => { vi.advanceTimersByTime(UNDO_WINDOW_MS + 10) })
    expect(screen.queryByRole('button', { name: '撤销' })).not.toBeInTheDocument()
  })

  it('shows a written draft as a card that opens it', async () => {
    const { client, view, onOpenDraft } = setup()
    client.getDraft.mockResolvedValue({ to: [{ name: '赵六', address: 'zhao@example.com' }] } as Draft)
    render(view(<ToolRow tool={tool({ name: 'draft_create', summary: { type: 'draft', draftId: 'd1', subject: '会议纪要' } })} />))
    expect(screen.getByText('会议纪要')).toBeInTheDocument()
    expect(await screen.findByText('赵六')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '打开草稿' }))
    expect(onOpenDraft).toHaveBeenCalledWith('d1')
  })
})

const turn = (over: Partial<ChatTurn>): ChatTurn => ({
  state: 'completed', text: '', tools: [], confirms: [], answers: {}, pendingId: null, error: null, finishedReason: 'stop', finishedAt: new Date(), ...over,
})
const reply = (over: Partial<ChatTurn>): ChatMessage => ({ id: 'r', role: 'assistant', turn: turn(over) })

describe('ChatMessages', () => {
  it('renders markdown as elements, shows HTML as text, and opens links outside the app', async () => {
    const { client, view } = setup()
    render(view(<ChatMessages messages={[
      { id: 'u', role: 'user', text: '你好' },
      reply({ text: '看 **重点**：\n\n- 第一项\n- 第二项\n\n<img src=x onerror=alert(1)> [文档](https://example.com/a) [坏](javascript:alert(1))' }),
    ]} />))
    expect(screen.getByRole('log')).toBeInTheDocument()
    expect(screen.getByText('重点').tagName).toBe('STRONG')
    expect(screen.getAllByRole('listitem')).toHaveLength(2)
    expect(document.querySelector('img')).toBeNull()
    expect(screen.getByText(/<img src=x onerror=alert\(1\)>/)).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: '坏' })).not.toBeInTheDocument()
    const link = screen.getByRole('link', { name: '文档' })
    expect(link).toHaveAttribute('title', 'https://example.com/a')
    await userEvent.click(link)
    expect(client.openExternalLink).toHaveBeenCalledWith('https://example.com/a')
  })

  it('says 已停止 for a stopped reply and offers 重试 only on the last failed one', async () => {
    const { view, retry } = setup()
    const { rerender } = render(view(<ChatMessages messages={[reply({ state: 'canceled', text: '写到一半' })]} />))
    expect(screen.getByText('已停止')).toBeInTheDocument()
    rerender(view(<ChatMessages messages={[reply({ state: 'failed', error: 'aiNetwork', finishedReason: null })]} />))
    expect(screen.getByText('连不上服务商')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(retry).toHaveBeenCalledTimes(1)
  })

  it('shows the thinking line before anything arrives', () => {
    const { view } = setup()
    render(view(<ChatMessages messages={[reply({ state: 'starting', finishedReason: null, finishedAt: null })]} />))
    expect(screen.getByText('正在思考…')).toBeInTheDocument()
  })

  it('answers a confirmation through the host and folds the card', async () => {
    const { view, respond } = setup()
    const request = { action: 'send' as const, tool: 'outbox_send', details: { type: 'outbox' as const, retry: false, senderAddress: 'me@example.com', subject: '周报', recipients: ['a@example.com'], recipientCount: 1 } }
    const waiting = reply({ state: 'awaitingConfirmation', confirms: [{ requestId: 'q', callId: 'k', request, at: 0 }], pendingId: 'q', finishedReason: null })
    const { rerender } = render(view(<ChatMessages messages={[waiting]} />))
    expect(screen.getByRole('group', { name: '取消发送？' })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '取消发送' }))
    expect(respond).toHaveBeenCalledWith(true)
    rerender(view(<ChatMessages messages={[reply({ confirms: [{ requestId: 'q', callId: 'k', request, at: 0 }], answers: { q: true } })]} />))
    expect(screen.getByText('已同意 · 已取消发送')).toBeInTheDocument()
  })
})

describe('message actions', () => {
  const at = new Date(2026, 10, 26, 9, 5).getTime()
  const actions = (row: HTMLElement) => within(within(row).getByRole('group', { name: '消息操作' }))

  it('copies a message and shows when it was said', async () => {
    const user = userEvent.setup()
    const { view } = setup()
    render(view(<ChatMessages messages={[{ id: 'u', role: 'user', text: '这周有哪些待办？', at }, reply({ text: '有两件。' })]} />))
    const question = actions(screen.getByText('这周有哪些待办？').parentElement!)
    expect(question.getByText('09:05')).toHaveAttribute('dateTime', new Date(at).toISOString())
    await user.click(question.getByRole('button', { name: '复制' }))
    expect(await navigator.clipboard.readText()).toBe('这周有哪些待办？')
    await user.click(actions(screen.getByText('有两件。').closest('.group') as HTMLElement).getByRole('button', { name: '复制' }))
    expect(await navigator.clipboard.readText()).toBe('有两件。')
  })

  it('rewords a message of yours in place, and only where nothing after it changed mail', async () => {
    const user = userEvent.setup()
    const { view, onEdit } = setup()
    const archived = reply({ text: '已归档', tools: [tool({ summary: { type: 'changed', count: 1, emailIds: ['e'], undo: null } as never })] })
    render(view(<ChatMessages messages={[
      { id: 'u0', role: 'user', text: '归档旧邮件', at }, { ...archived, id: 'r0' },
      { id: 'u1', role: 'user', text: '总结一下', at }, reply({ text: '好的' }),
    ]} />))
    // Before the archive: no edit, since what it did would stay done.
    expect(actions(screen.getByText('归档旧邮件').parentElement!).queryByRole('button', { name: '编辑' })).toBeNull()
    await user.click(actions(screen.getByText('总结一下').parentElement!).getByRole('button', { name: '编辑' }))
    const field = screen.getByRole('textbox', { name: '编辑消息' })
    expect(field).toHaveValue('总结一下')
    expect(screen.getByRole('button', { name: '发送' })).toBeDisabled()
    await user.type(field, '，简短些{Enter}')
    expect(onEdit).toHaveBeenCalledWith('u1', '总结一下，简短些')
    expect(screen.queryByRole('textbox', { name: '编辑消息' })).not.toBeInTheDocument()
    await user.click(actions(screen.getByText('总结一下').parentElement!).getByRole('button', { name: '编辑' }))
    await user.keyboard('{Escape}')
    expect(screen.getByText('总结一下')).toBeInTheDocument()
    expect(onEdit).toHaveBeenCalledTimes(1)
  })

  it('regenerates only the last reply, and never one that changed mail', async () => {
    const { view, regenerate } = setup()
    const { rerender } = render(view(<ChatMessages messages={[
      { ...reply({ text: '第一次' }), id: 'r0' }, { id: 'u', role: 'user', text: '再说说' }, reply({ text: '第二次' }),
    ]} />))
    expect(screen.getAllByRole('button', { name: '重新生成' })).toHaveLength(1)
    await userEvent.click(screen.getByRole('button', { name: '重新生成' }))
    expect(regenerate).toHaveBeenCalledTimes(1)
    rerender(view(<ChatMessages messages={[reply({ text: '已归档', tools: [tool({ summary: { type: 'changed', count: 2 } as never })] })]} />))
    expect(screen.queryByRole('button', { name: '重新生成' })).not.toBeInTheDocument()
  })

  it('goes on with a reply that stopped short, and offers nothing while one is written', async () => {
    const { view, continueReply } = setup()
    const { rerender } = render(view(<ChatMessages messages={[reply({ text: '完整的回答' })]} />))
    expect(screen.queryByRole('button', { name: '继续生成' })).not.toBeInTheDocument()
    rerender(view(<ChatMessages messages={[reply({ state: 'canceled', text: '写到一半' })]} />))
    await userEvent.click(screen.getByRole('button', { name: '继续生成' }))
    expect(continueReply).toHaveBeenCalledTimes(1)
    rerender(view(<ChatMessages messages={[reply({ state: 'running', text: '正在写', finishedReason: null, finishedAt: null })]} />))
    expect(screen.queryByRole('group', { name: '消息操作' })).not.toBeInTheDocument()
  })
})
