import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AiState, AssistantEvent, AssistantEventKind, ConversationDetail, ConversationSummary, ConversationsChanged, EmailPage, MailOverview, RunSnapshot, StartRun } from '@/generated/accounts'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { TooltipProvider } from '@/components/ui/tooltip'
import { AssistantChatProvider } from '@/features/ai/assistant-chat-provider'
import { NowPage, type NowPageProps } from './now-page'

const account = exampleAccount({ displayName: '工作邮箱', senderName: '林夕' })
const empty: MailOverview = { newMail: { total: 0, accounts: [], latest: [], senders: [] }, starredUnread: 0, unreadSenders: [], verificationCodes: [], codeCandidates: [], recentAttachments: [], generatedAt: new Date().toISOString() }
const noMail: EmailPage = { items: [], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false, historyReachedEnd: false } }
const CONNECTION = { id: 'c1', providerId: 'openai', name: 'OpenAI', protocol: 'openaiCompatible' as const, baseUrl: 'https://api.openai.com/v1', useProxy: false, hasKey: true }
const ready: AiState = {
  connections: [CONNECTION], defaultModel: { connectionId: 'c1', modelId: 'big', name: 'Big', contextLength: null, outputLength: null },
  translationModel: null, route: { kind: 'direct' }, proxyPasswordSet: false,
}

const snapshot = (over: Partial<RunSnapshot> = {}): RunSnapshot => ({
  runId: 'chat1', kind: 'chat', state: 'running', text: '', tools: [], reasoning: [], toolsAvailable: true, error: null, finishedReason: null, completedAt: null, seq: 1, conversationId: 'conv-live', pending: null, ...over,
})
const event = (seq: number, kind: AssistantEventKind, runId = 'chat1'): AssistantEvent => ({ runId, seq, kind })
const done = (from: number) => [event(from, { type: 'finished', reason: 'stop' }), event(from + 1, { type: 'state', state: 'completed' })]

const hoursAgo = (hours: number) => new Date(Date.now() - hours * 3_600_000).toISOString()
const daysAgo = (days: number) => new Date(Date.now() - days * 86_400_000).toISOString()
const item = (id: string, title: string, updatedAt: string): ConversationSummary => ({ id, title, updatedAt, messageCount: 1 })

const PAST: ConversationDetail = {
  id: 'conv-old', title: '归档推广邮件', createdAt: daysAgo(3), updatedAt: daysAgo(3),
  turns: [
    { role: 'user', text: '把推广邮件归档', at: daysAgo(3) },
    {
      role: 'assistant', text: '先搜索，然后已经归档了。', state: 'completed', finishedReason: 'stop', error: null, at: daysAgo(3),
      tools: [
        { callId: 'a', name: 'mail_search', status: 'done', summary: { type: 'found', count: 3 }, textOffset: 0 },
        { callId: 'b', name: 'mail_archive', status: 'done', summary: { type: 'changed', count: 2, emailIds: [], undo: null }, textOffset: 4 },
      ],
      reasoning: [],
      confirmations: [{ callId: 'b', tool: 'mail_archive', action: 'organize', outcome: 'approved', textOffset: 4 }],
    },
    { role: 'user', text: '再发一封', at: daysAgo(3) },
    { role: 'assistant', text: '', state: 'failed', finishedReason: null, error: 'aiUnavailable', tools: [], reasoning: [], confirmations: [], at: daysAgo(3) },
  ],
}

type Setup = { items?: ConversationSummary[]; pages?: Array<{ items: ConversationSummary[]; nextCursor: string | null }>; saveHistory?: boolean }

/** The Now page with a conversation already on screen, over a client that serves saved conversations. */
async function setup({ items = [], pages, saveHistory = true }: Setup = {}) {
  const client = testClient([account])
  client.getMailOverview.mockResolvedValue(empty)
  client.listEmails.mockResolvedValue(noMail)
  client.aiState.mockResolvedValue(ready)
  if (!saveHistory) {
    const read = client.getSettings.getMockImplementation()!
    client.getSettings.mockImplementation(async () => {
      const current = await read()
      return { ...current, settings: { ...current.settings, assistant: { ...current.settings.assistant, saveHistory: false } } }
    })
  }
  const listeners = new Set<(event: AssistantEvent) => void>()
  client.onAssistantEvent.mockImplementation((listener) => { listeners.add(listener); return () => { listeners.delete(listener) } })
  const changes = new Set<(changed: ConversationsChanged) => void>()
  client.onAssistantConversationsChanged.mockImplementation((listener) => { changes.add(listener); return () => { changes.delete(listener) } })
  const queue = pages ? [...pages] : [{ items, nextCursor: null }]
  client.assistantListConversations.mockImplementation(async (page) => {
    if (pages) return page?.cursor ? pages[1] : pages[0]
    return queue[0]
  })
  client.assistantGetConversation.mockImplementation(async (id) => id === PAST.id ? PAST : { ...PAST, id, title: id, turns: PAST.turns.slice(0, 2) })
  client.assistantStart.mockImplementation(async ({ kind }: StartRun) => {
    if (kind.type === 'nowSummary') return snapshot({ runId: 'sum', kind: 'nowSummary', state: 'completed' })
    return snapshot({ conversationId: kind.type === 'chat' ? kind.conversationId ?? 'conv-live' : null })
  })
  const props: NowPageProps = {
    client, accounts: [account], folders: [], syncStates: {}, draftCount: 0, outbox: [],
    verification: { status: () => undefined, busy: () => false, copied: () => false, copy: vi.fn(), requestVisible: () => undefined },
    onSelectMailbox: vi.fn(), onOpenUnread: vi.fn(), onOpenAccount: vi.fn(), onOpenContact: vi.fn(), onOpenEmail: vi.fn(), onOpenDraft: vi.fn(),
  }
  render(<TooltipProvider><AssistantChatProvider client={client}><NowPage {...props} /></AssistantChatProvider></TooltipProvider>)
  const emit = (...events: AssistantEvent[]) => act(() => { for (const next of events) for (const listener of [...listeners]) listener(next) })
  const changed = (value: ConversationsChanged) => act(() => { for (const listener of [...changes]) listener(value) })
  await userEvent.type(await screen.findByRole('textbox', { name: '问问你的邮件' }), '你好{Enter}')
  await emit(event(2, { type: 'text', delta: '你好。' }), ...done(3))
  await screen.findByText('你好。')
  return { client, emit, changed, queue }
}

const openPanel = async () => {
  await userEvent.click(screen.getByRole('button', { name: '历史对话' }))
  return screen.findByRole('dialog', { name: '历史对话' })
}

afterEach(() => { vi.restoreAllMocks() })

describe('history button and panel', () => {
  it('sits in the conversation header after 新对话, and opens a panel grouped by day with the current one marked', async () => {
    await setup({ items: [item('conv-live', '你好', hoursAgo(0.01)), item('conv-y', '昨天的事', daysAgo(1)), item('conv-old', '很久以前', daysAgo(5))] })
    const header = screen.getByRole('button', { name: '新对话' }).parentElement!
    expect([...header.querySelectorAll('button')].map((button) => button.getAttribute('aria-label') ?? button.textContent)).toEqual(['返回', '新对话', '历史对话'])
    const panel = await openPanel()
    await within(panel).findByText('很久以前')
    expect(within(panel).getAllByRole('heading').map((heading) => heading.textContent)).toEqual(['今天', '昨天', '更早'])
    const current = within(panel).getByRole('button', { name: /你好/ })
    expect(current).toHaveAttribute('aria-current', 'true')
    expect(within(panel).getByRole('button', { name: /昨天的事/ })).not.toHaveAttribute('aria-current')
    expect(panel).toHaveTextContent('刚刚')
    expect(panel).not.toHaveTextContent('未保存对话记录')
  })

  it('opens a past conversation with its tools and answered confirmation in place, no undo, and continues it', async () => {
    const { client } = await setup({ items: [item('conv-old', '归档推广邮件', daysAgo(3))] })
    const panel = await openPanel()
    await userEvent.click(await within(panel).findByRole('button', { name: /归档推广邮件/ }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '历史对话' })).not.toBeInTheDocument())
    expect(client.assistantGetConversation).toHaveBeenCalledWith('conv-old')
    const log = await screen.findByRole('log')
    await within(log).findByText('把推广邮件归档')
    expect(screen.queryByText('你好。')).not.toBeInTheDocument()
    // Offset 0 puts the search first; offset 4 puts the answered confirmation after "先搜索，".
    const slot = log.querySelector('[data-slot="chat-assistant"]')!
    const parts = [...slot.children].map((child) => child.getAttribute('data-slot') ?? '').filter((name) => name !== '')
    expect(parts.slice(0, 4)).toEqual(['tool-row', 'markdown', 'confirm-card', 'markdown'])
    expect(slot).toHaveTextContent('搜索了 3 封邮件')
    expect(slot.querySelectorAll('[data-slot="tool-row"]')).toHaveLength(1)
    expect(slot.querySelector('[data-slot="confirm-card"]')).toHaveTextContent('已同意 · 已整理')
    expect(within(log).queryByRole('button', { name: '撤销' })).not.toBeInTheDocument()
    // A failed reply read back shows its reason but no retry: its words are already in the conversation.
    expect(log).toHaveTextContent('再发一封')
    expect(within(log).queryByRole('button', { name: '重试' })).not.toBeInTheDocument()
    // The next message continues that conversation.
    await userEvent.type(screen.getByRole('textbox', { name: '问问你的邮件' }), '继续{Enter}')
    await waitFor(() => expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', conversationId: 'conv-old', message: '继续' } }))
    expect(within(screen.getByRole('log')).getByText('把推广邮件归档')).toBeInTheDocument()
  })

  it('puts tools where their offsets say, between the pieces of text', async () => {
    const { client } = await setup({ items: [item('conv-old', '归档推广邮件', daysAgo(3))] })
    client.assistantGetConversation.mockResolvedValue({
      ...PAST,
      turns: [
        { role: 'user', text: '查一下', at: daysAgo(1) },
        { role: 'assistant', text: '先看看。结果如下。', state: 'completed', finishedReason: 'stop', error: null, at: daysAgo(1), confirmations: [], reasoning: [], tools: [{ callId: 'a', name: 'mail_search', status: 'done', summary: { type: 'found', count: 7 }, textOffset: 4 }] },
      ],
    })
    const panel = await openPanel()
    await userEvent.click(await within(panel).findByRole('button', { name: /归档推广邮件/ }))
    const slot = (await screen.findByText('查一下')) && document.querySelector('[data-slot="chat-assistant"]')!
    await waitFor(() => expect(slot).toHaveTextContent('结果如下'))
    const kids = [...slot.children].map((child) => child.textContent ?? '')
    const at = (text: string) => kids.findIndex((value) => value.includes(text))
    expect(at('先看看')).toBeLessThan(at('搜索了 7 封邮件'))
    expect(at('搜索了 7 封邮件')).toBeLessThan(at('结果如下'))
  })

  it('stops a running reply when another conversation is opened', async () => {
    const { client, emit } = await setup({ items: [item('conv-old', '归档推广邮件', daysAgo(3))] })
    client.assistantStart.mockResolvedValueOnce(snapshot({ runId: 'chat2', conversationId: 'conv-live' }))
    await userEvent.type(screen.getByRole('textbox', { name: '问问你的邮件' }), '再来{Enter}')
    await emit(event(2, { type: 'text', delta: '写到一半' }, 'chat2'))
    const panel = await openPanel()
    await userEvent.click(await within(panel).findByRole('button', { name: /归档推广邮件/ }))
    await waitFor(() => expect(client.assistantCancel).toHaveBeenCalledWith('chat2'))
    await screen.findByText('把推广邮件归档')
    expect(screen.queryByText('写到一半')).not.toBeInTheDocument()
  })

  it('deletes with a small question in the row, not a dialog', async () => {
    const { client } = await setup({ items: [item('conv-a', '第一段', hoursAgo(1)), item('conv-b', '第二段', hoursAgo(2))] })
    const panel = await openPanel()
    await within(panel).findByText('第二段')
    await userEvent.click(within(panel).getAllByRole('button', { name: '删除对话' })[1])
    expect(within(panel).getByText('删除？')).toBeInTheDocument()
    expect(client.assistantDeleteConversation).not.toHaveBeenCalled()
    await userEvent.click(within(panel).getByRole('button', { name: '取消' }))
    expect(within(panel).getByText('第二段')).toBeInTheDocument()
    await userEvent.click(within(panel).getAllByRole('button', { name: '删除对话' })[1])
    await userEvent.click(within(panel).getByRole('button', { name: '确定' }))
    await waitFor(() => expect(client.assistantDeleteConversation).toHaveBeenCalledWith('conv-b'))
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
  })

  it('goes to an empty conversation when the one on screen is deleted', async () => {
    const { client } = await setup({ items: [item('conv-live', '你好', hoursAgo(0.01))] })
    const panel = await openPanel()
    await within(panel).findByText('你好')
    await userEvent.click(within(panel).getByRole('button', { name: '删除对话' }))
    await userEvent.click(within(panel).getByRole('button', { name: '确定' }))
    await waitFor(() => expect(client.assistantDeleteConversation).toHaveBeenCalledWith('conv-live'))
    // Back on the overview: the conversation header is gone; the overview has its own history button.
    await waitFor(() => expect(screen.queryByRole('button', { name: '返回' })).not.toBeInTheDocument())
    expect(await screen.findByRole('heading', { level: 1 })).toBeInTheDocument()
  })

  it('clears everything after an inline question', async () => {
    const { client } = await setup({ items: [item('conv-a', '第一段', hoursAgo(1))] })
    const panel = await openPanel()
    await within(panel).findByText('第一段')
    await userEvent.click(within(panel).getByRole('button', { name: '清除全部' }))
    expect(within(panel).getByText('清除全部对话？')).toBeInTheDocument()
    expect(client.assistantClearConversations).not.toHaveBeenCalled()
    await userEvent.click(within(panel).getByRole('button', { name: '确定' }))
    await waitFor(() => expect(client.assistantClearConversations).toHaveBeenCalledTimes(1))
  })

  it('loads more when the core has more, and refreshes when it says the list changed', async () => {
    const { client, changed } = await setup({
      pages: [{ items: [item('c1', '第一页', hoursAgo(1))], nextCursor: 'next' }, { items: [item('c2', '第二页', hoursAgo(2))], nextCursor: null }],
    })
    const panel = await openPanel()
    await within(panel).findByText('第一页')
    expect(client.assistantListConversations).toHaveBeenCalledWith({ limit: 30, cursor: null })
    await userEvent.click(within(panel).getByRole('button', { name: '加载更多' }))
    await within(panel).findByText('第二页')
    expect(client.assistantListConversations).toHaveBeenLastCalledWith({ limit: 30, cursor: 'next' })
    expect(within(panel).queryByRole('button', { name: '加载更多' })).not.toBeInTheDocument()
    const before = client.assistantListConversations.mock.calls.length
    await changed({ conversationId: 'c3', change: 'saved' })
    await waitFor(() => expect(client.assistantListConversations.mock.calls.length).toBeGreaterThan(before))
  })

  it('says so when there is nothing yet', async () => {
    await setup({ items: [] })
    const panel = await openPanel()
    expect(await within(panel).findByText('还没有历史对话')).toBeInTheDocument()
    expect(within(panel).queryByRole('button', { name: '清除全部' })).not.toBeInTheDocument()
  })

  it('moves with the arrow keys, opens with Enter, asks with Delete, and Esc closes and returns to the button', async () => {
    const { client } = await setup({ items: [item('conv-a', '第一段', hoursAgo(1)), item('conv-b', '第二段', hoursAgo(2))] })
    const button = screen.getByRole('button', { name: '历史对话' })
    const panel = await openPanel()
    const first = await within(panel).findByRole('button', { name: /第一段/ })
    const second = within(panel).getByRole('button', { name: /第二段/ })
    first.focus()
    await userEvent.keyboard('{ArrowDown}')
    expect(second).toHaveFocus()
    await userEvent.keyboard('{ArrowUp}')
    expect(first).toHaveFocus()
    await userEvent.keyboard('{ArrowDown}{Delete}')
    expect(within(panel).getByText('删除？')).toBeInTheDocument()
    // Esc takes back the question first.
    await userEvent.keyboard('{Escape}')
    expect(within(panel).queryByText('删除？')).not.toBeInTheDocument()
    expect(screen.getByRole('dialog', { name: '历史对话' })).toBeInTheDocument()
    // The row came back as a new element, with the focus on it.
    expect(within(panel).getByRole('button', { name: /第二段/ })).toHaveFocus()
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(client.assistantGetConversation).toHaveBeenCalledWith('conv-b'))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '历史对话' })).not.toBeInTheDocument())
    await userEvent.click(button)
    await screen.findByRole('dialog', { name: '历史对话' })
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '历史对话' })).not.toBeInTheDocument())
    expect(button).toHaveFocus()
    // The conversation is still there: Esc closed the panel, not the page's conversation.
    expect(screen.getByRole('button', { name: '返回' })).toBeInTheDocument()
  })

  it('says chats are not saved when the setting is off, and the link opens Settings → AI', async () => {
    await setup({ items: [], saveHistory: false })
    const requested: unknown[] = []
    const listen = (event: Event) => requested.push((event as CustomEvent).detail)
    window.addEventListener('mailx:open-settings', listen)
    const panel = await openPanel()
    expect(await within(panel).findByText('未保存对话记录')).toBeInTheDocument()
    await userEvent.click(within(panel).getByRole('button', { name: '开启' }))
    window.removeEventListener('mailx:open-settings', listen)
    expect(requested).toEqual([{ section: 'ai' }])
  })
})
