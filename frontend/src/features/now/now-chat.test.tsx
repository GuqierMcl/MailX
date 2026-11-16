import { useState } from 'react'
import { act, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AiState, AssistantEvent, AssistantEventKind, ConfirmationRequest, EmailPage, MailOverview, RunSnapshot, StartRun } from '@/generated/accounts'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { TooltipProvider } from '@/components/ui/tooltip'
import { AssistantChatProvider } from '@/features/ai/assistant-chat-provider'
import { NowPage, type NowPageProps } from './now-page'

const account = exampleAccount({ displayName: '工作邮箱', senderName: '林夕' })
const empty: MailOverview = { newMail: { total: 0, accounts: [], latest: [] }, starredUnread: 0, verificationCodes: [], codeCandidates: [], recentAttachments: [], generatedAt: new Date().toISOString() }
const noMail: EmailPage = { items: [], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }
const CONNECTION = { id: 'c1', providerId: 'openai', name: 'OpenAI', protocol: 'openaiCompatible' as const, baseUrl: 'https://api.openai.com/v1', useProxy: false, hasKey: true }
const ready: AiState = {
  connections: [CONNECTION], defaultModel: { connectionId: 'c1', modelId: 'big', name: 'Big', contextLength: null, outputLength: null },
  translationModel: null, route: { kind: 'direct' }, proxyPasswordSet: false,
}
const SEND: ConfirmationRequest = {
  action: 'send', tool: 'send_email',
  details: { type: 'send', accountName: '工作邮箱', accountAddress: 'me@example.com', to: [{ name: null, address: 'a@example.com' }], cc: [], bcc: [], subject: '周报', bodyPreview: '正文', attachments: [], warnings: ['newRecipient'] },
}

const snapshot = (over: Partial<RunSnapshot> = {}): RunSnapshot => ({
  runId: 'chat1', kind: 'chat', state: 'running', text: '', tools: [], toolsAvailable: true, error: null, finishedReason: null, completedAt: null, seq: 1, conversationId: 'conv1', pending: null, ...over,
})
const event = (seq: number, kind: AssistantEventKind, runId = 'chat1'): AssistantEvent => ({ runId, seq, kind })
const done = (from: number, runId = 'chat1') => [event(from, { type: 'finished', reason: 'stop' }, runId), event(from + 1, { type: 'state', state: 'completed' }, runId)]

function Harness({ props }: { props: NowPageProps }) {
  const [open, setOpen] = useState(true)
  return <>
    <button type="button" onClick={() => setOpen((value) => !value)}>toggle page</button>
    {open && <NowPage {...props} />}
  </>
}

type Setup = { ai?: AiState; enabled?: boolean; chat?: Array<RunSnapshot | Error>; withProvider?: boolean }

/** The Now page over a client whose chat events the test sends by hand; the summary run just completes. */
function setup({ ai = ready, enabled = true, chat = [snapshot()], withProvider = true }: Setup = {}) {
  const client = testClient([account])
  client.getMailOverview.mockResolvedValue(empty)
  client.listEmails.mockResolvedValue(noMail)
  client.aiState.mockResolvedValue(ai)
  if (!enabled) {
    const read = client.getSettings.getMockImplementation()!
    client.getSettings.mockImplementation(async () => {
      const current = await read()
      return { ...current, settings: { ...current.settings, assistant: { ...current.settings.assistant, enabled: false } } }
    })
  }
  const listeners = new Set<(event: AssistantEvent) => void>()
  client.onAssistantEvent.mockImplementation((listener) => { listeners.add(listener); return () => { listeners.delete(listener) } })
  const queue = [...chat]
  client.assistantStart.mockImplementation(async ({ kind }: StartRun) => {
    if (kind.type === 'nowSummary') return snapshot({ runId: 'sum', kind: 'nowSummary', state: 'completed', text: '' })
    const next = queue.length > 1 ? queue.shift()! : queue[0]
    if (next instanceof Error) throw next
    return next
  })
  const props: NowPageProps = {
    client, accounts: [account], folders: [], syncStates: {}, draftCount: 0, outbox: [],
    verification: { status: () => undefined, busy: () => false, copied: () => false, copy: vi.fn(), requestVisible: () => undefined },
    onSelectMailbox: vi.fn(), onOpenUnread: vi.fn(), onOpenContact: vi.fn(), onOpenAccount: vi.fn(), onOpenEmail: vi.fn(), onOpenDraft: vi.fn(),
  }
  const page = <Harness props={props} />
  render(<TooltipProvider>{withProvider ? <AssistantChatProvider client={client}>{page}</AssistantChatProvider> : page}</TooltipProvider>)
  const emit = (...events: AssistantEvent[]) => act(() => { for (const next of events) for (const listener of [...listeners]) listener(next) })
  return { client, props, emit }
}

const input = () => screen.findByRole('textbox', { name: '问问你的邮件' })
const chatStarts = (client: ReturnType<typeof setup>['client']) => client.assistantStart.mock.calls.filter(([request]) => request.kind.type === 'chat').map(([request]) => request.kind)
const ask = async (text: string) => {
  await userEvent.type(await input(), `${text}{Enter}`)
}
const greeting = () => screen.queryByRole('heading', { level: 1 })

afterEach(() => {
  localStorage.clear()
  vi.restoreAllMocks()
})

describe('Now page: assistant input', () => {
  it('shows the input only while the assistant is usable', async () => {
    const off = setup({ enabled: false })
    await screen.findByRole('heading', { level: 1 })
    expect(screen.queryByRole('textbox', { name: '问问你的邮件' })).not.toBeInTheDocument()
    expect(screen.queryByText(/配置|设置/)).not.toBeInTheDocument()
    expect(off.client.assistantStart).not.toHaveBeenCalled()
  })

  it('stays away without a model', async () => {
    setup({ ai: { ...ready, defaultModel: null } })
    await screen.findByRole('heading', { level: 1 })
    expect(screen.queryByRole('textbox', { name: '问问你的邮件' })).not.toBeInTheDocument()
  })

  it('stays away where no conversation is kept', async () => {
    setup({ withProvider: false })
    await screen.findByRole('heading', { level: 1 })
    expect(screen.queryByRole('textbox', { name: '问问你的邮件' })).not.toBeInTheDocument()
  })

  it('appears at the bottom of the page once the assistant is usable', async () => {
    setup()
    expect(await input()).toBeInTheDocument()
    expect(document.querySelector('[data-slot="chat-footer"]')).toBeInTheDocument()
  })
})

describe('Now page: conversation', () => {
  it('turns the page into the conversation after the first message and streams the reply', async () => {
    const { client, emit } = setup()
    await screen.findByRole('heading', { level: 1 })
    await ask('我有什么新邮件')
    expect(chatStarts(client)).toEqual([{ type: 'chat', message: '我有什么新邮件' }])
    expect(await screen.findByRole('button', { name: '返回' })).toBeInTheDocument()
    expect(greeting()).not.toBeInTheDocument()
    expect(screen.getByText('我有什么新邮件')).toBeInTheDocument()
    expect(screen.getByText('正在思考…')).toBeInTheDocument()
    await emit(event(2, { type: 'text', delta: '有 **两封**：\n\n- 账单\n- 通知' }), ...done(3))
    expect(await screen.findByText('两封')).toBeInTheDocument()
    expect(screen.getAllByRole('listitem')).toHaveLength(2)
    // The summary line keeps the greeting.
    expect(screen.getByRole('button', { name: '返回' }).parentElement).toHaveTextContent('林夕')
  })

  it('goes back to the overview with the button and Esc, and keeps the conversation', async () => {
    const { emit } = setup()
    await ask('你好')
    await emit(event(2, { type: 'text', delta: '你好。' }), ...done(3))
    await screen.findByText('你好。')
    await userEvent.click(screen.getByRole('button', { name: '返回' }))
    expect(await screen.findByRole('heading', { level: 1 })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '回到对话' }))
    expect(await screen.findByText('你好。')).toBeInTheDocument()
    // Esc in the field, nothing running: back to the overview.
    await userEvent.type(await input(), '{Escape}')
    expect(await screen.findByRole('heading', { level: 1 })).toBeInTheDocument()
  })

  it('stops a running reply with Esc before it leaves', async () => {
    const { client } = setup()
    await ask('长一点')
    await screen.findByRole('button', { name: '返回' })
    await userEvent.type(await input(), '{Escape}')
    expect(client.assistantCancel).toHaveBeenCalledWith('chat1')
    expect(screen.getByRole('button', { name: '返回' })).toBeInTheDocument()
  })

  it('keeps the conversation when the page is closed and opened again', async () => {
    const { emit } = setup()
    await ask('记住我')
    await emit(event(2, { type: 'text', delta: '记住了。' }), ...done(3))
    await screen.findByText('记住了。')
    await userEvent.click(screen.getByRole('button', { name: 'toggle page' }))
    await userEvent.click(screen.getByRole('button', { name: 'toggle page' }))
    expect(await screen.findByRole('heading', { level: 1 })).toBeInTheDocument()
    await userEvent.click(await screen.findByRole('button', { name: '回到对话' }))
    expect(await screen.findByText('记住了。')).toBeInTheDocument()
    expect(screen.getByText('记住我')).toBeInTheDocument()
  })

  it('starts a new conversation on request', async () => {
    const { client, emit } = setup({ chat: [snapshot(), snapshot({ runId: 'chat2', conversationId: 'conv2' })] })
    await ask('第一个')
    await emit(...done(2))
    await userEvent.click(await screen.findByRole('button', { name: '新对话' }))
    expect(await screen.findByRole('heading', { level: 1 })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '回到对话' })).not.toBeInTheDocument()
    await ask('第二个')
    expect(chatStarts(client)).toEqual([{ type: 'chat', message: '第一个' }, { type: 'chat', message: '第二个' }])
  })

  it('shows the quiet hint when the model cannot work with mail', async () => {
    const { client, emit } = setup()
    client.assistantSnapshot.mockResolvedValue(snapshot({ state: 'completed', toolsAvailable: false }))
    await ask('你好')
    expect(screen.queryByText('当前模型不支持操作邮件，只能回答问题。')).not.toBeInTheDocument()
    await emit(...done(2))
    expect(await screen.findByText('当前模型不支持操作邮件，只能回答问题。')).toBeInTheDocument()
  })

  it('opens an email or a draft the reply points to', async () => {
    const { client, props, emit } = setup()
    client.getEmail.mockResolvedValue({
      summary: { id: 'm1', accountId: account.id, subject: '账单', from: { name: '银行', address: 'bank@example.com' }, receivedAt: '2026-11-16T08:00:00Z' },
      to: [], cc: [], replyTo: [], body: null, bodyFailure: null, attachments: [],
    } as never)
    client.getDraft.mockRejectedValue(new Error('no'))
    await ask('看看')
    await emit(
      event(2, { type: 'tool', callId: 'k1', name: 'read_email', status: 'done', summary: { type: 'read', emailIds: ['m1'] } }),
      event(3, { type: 'tool', callId: 'k2', name: 'create_draft', status: 'done', summary: { type: 'draft', draftId: 'd1', subject: '回复账单' } }),
      ...done(4),
    )
    await userEvent.click(await screen.findByRole('button', { name: '展开详情' }))
    await userEvent.click(await screen.findByRole('button', { name: /银行 · 账单/ }))
    expect(props.onOpenEmail).toHaveBeenCalledWith({ id: 'm1', accountId: account.id })
    await userEvent.click(screen.getByRole('button', { name: '打开草稿' }))
    expect(props.onOpenDraft).toHaveBeenCalledWith('d1')
  })
})

describe('Now page: consent', () => {
  it('asks first, then sends the same message once the user agrees', async () => {
    const { client } = setup({ chat: [snapshot({ state: 'awaitingConsent', conversationId: null, runId: 'wait' }), snapshot()] })
    await ask('先问我')
    expect(await screen.findByText('把邮件内容发送给OpenAI？')).toBeInTheDocument()
    // Nothing of the conversation shows yet.
    expect(screen.queryByRole('button', { name: '返回' })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '同意' }))
    expect(client.assistantGrantConsent).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(chatStarts(client)).toEqual([{ type: 'chat', message: '先问我' }, { type: 'chat', message: '先问我' }]))
    expect(await screen.findByRole('button', { name: '返回' })).toBeInTheDocument()
    expect(screen.getByText('先问我')).toBeInTheDocument()
  })

  it('puts the words back in the field when the user declines', async () => {
    const { client } = setup({ chat: [snapshot({ state: 'awaitingConsent', conversationId: null, runId: 'wait' })] })
    await ask('不同意')
    await userEvent.click(await screen.findByRole('button', { name: '取消' }))
    expect(client.assistantGrantConsent).not.toHaveBeenCalled()
    await waitFor(() => expect(screen.getByRole('textbox', { name: '问问你的邮件' })).toHaveValue('不同意'))
  })
})

describe('Now page: confirmation', () => {
  const ask2 = async (emit: ReturnType<typeof setup>['emit']) => {
    await ask('发出去')
    await emit(
      event(2, { type: 'text', delta: '草稿好了。' }),
      event(3, { type: 'tool', callId: 'k1', name: 'send_email', status: 'running', summary: { type: 'pending' } }),
      event(4, { type: 'state', state: 'awaitingConfirmation' }),
      event(5, { type: 'confirm', requestId: 'q1', callId: 'k1', request: SEND }),
    )
  }

  it('asks on a card, sends on 发送 and folds to a line', async () => {
    const { client, emit } = setup()
    await ask2(emit)
    const card = await screen.findByRole('group', { name: '发送这封邮件？' })
    expect(card).toHaveTextContent('第一次联系这个地址')
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(client.assistantRespond).toHaveBeenCalledWith('chat1', 'q1', true)
    await emit(event(6, { type: 'state', state: 'running' }))
    expect(await screen.findByText('已同意 · 发送已排队')).toBeInTheDocument()
    // The field gets the focus back.
    await waitFor(() => expect(screen.getByRole('textbox', { name: '问问你的邮件' })).toHaveFocus())
  })

  it('folds to 已取消 on 取消', async () => {
    const { client, emit } = setup()
    await ask2(emit)
    await userEvent.click(await screen.findByRole('button', { name: '取消' }))
    expect(client.assistantRespond).toHaveBeenCalledWith('chat1', 'q1', false)
    await emit(event(6, { type: 'state', state: 'running' }))
    expect(await screen.findByText('已取消')).toBeInTheDocument()
  })

  it('points to a waiting card from the overview', async () => {
    const { emit } = setup()
    await ask2(emit)
    await screen.findByRole('group', { name: '发送这封邮件？' })
    await userEvent.click(screen.getByRole('button', { name: '返回' }))
    await screen.findByRole('heading', { level: 1 })
    const pill = await screen.findByRole('button', { name: '有一项操作等待你确认' })
    await userEvent.click(pill)
    expect(await screen.findByRole('group', { name: '发送这封邮件？' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '返回' })).toBeInTheDocument()
  })
})

describe('Now page: swapping views', () => {
  const overview = () => document.querySelector('[data-slot="now-overview"]') as HTMLElement
  const conversation = () => document.querySelector('[data-slot="now-conversation"]') as HTMLElement | null
  const animating = (node: Element | null) => node ? /animate-view-/.test(node.className) || node.className.includes('will-change') : false

  it('moves the live views with classes, never with a view transition', async () => {
    const start = vi.fn()
    Object.defineProperty(document, 'startViewTransition', { configurable: true, value: start })
    try {
      const { emit } = setup()
      await screen.findByRole('heading', { level: 1 })
      expect(animating(overview())).toBe(false)
      await ask('你好')
      await screen.findByRole('button', { name: '返回' })
      // During the swap: the overview leaves upward, the conversation comes up from below, neither takes clicks.
      expect(overview()).toHaveAttribute('data-phase', 'to-conversation')
      expect(overview()).toHaveClass('animate-view-out-up', 'pointer-events-none', 'will-change-[transform,opacity]')
      expect(overview()).toHaveAttribute('inert')
      expect(conversation()).toHaveClass('animate-view-in-up', 'pointer-events-none')
      expect(document.documentElement.dataset.nowTransition).toBeUndefined()
      // The input is outside both, so it is not part of the motion.
      expect(document.querySelector('[data-slot="chat-footer"]')?.closest('[data-slot="now-overview"], [data-slot="now-conversation"]')).toBeNull()
      // Afterwards: nothing animates, no will-change, the overview is hidden from everyone.
      await waitFor(() => expect(overview()).not.toHaveAttribute('data-phase'))
      expect(animating(overview())).toBe(false)
      expect(animating(conversation())).toBe(false)
      expect(overview()).toHaveClass('invisible')
      expect(overview()).toHaveAttribute('inert')
      expect(overview()).toHaveAttribute('aria-hidden', 'true')
      await emit(...done(2))

      // And back.
      await userEvent.click(screen.getByRole('button', { name: '返回' }))
      expect(conversation()).toHaveAttribute('data-phase', 'to-overview')
      expect(conversation()).toHaveClass('animate-view-out-down', 'pointer-events-none')
      expect(conversation()).toHaveAttribute('inert')
      expect(overview()).toHaveClass('animate-view-in-down')
      expect(overview()).not.toHaveAttribute('inert')
      await waitFor(() => expect(conversation()).toBeNull())
      expect(animating(overview())).toBe(false)
      expect(overview()).not.toHaveClass('invisible')
      expect(start).not.toHaveBeenCalled()
    } finally {
      delete (document as unknown as Record<string, unknown>).startViewTransition
    }
  })

  it('swaps at once with reduced motion', async () => {
    vi.spyOn(window, 'matchMedia').mockImplementation((query: string) => ({
      matches: query.includes('reduce'), media: query, onchange: null,
      addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
    }))
    setup()
    await screen.findByRole('heading', { level: 1 })
    await ask('你好')
    await screen.findByRole('button', { name: '返回' })
    expect(overview()).not.toHaveAttribute('data-phase')
    expect(animating(overview())).toBe(false)
    expect(animating(conversation())).toBe(false)
    expect(overview()).toHaveClass('invisible')
    await userEvent.click(screen.getByRole('button', { name: '返回' }))
    expect(conversation()).toBeNull()
    expect(animating(overview())).toBe(false)
    expect(overview()).not.toHaveClass('invisible')
  })
})
