import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook, waitFor } from '@/test/render'
import { testClient } from '@/features/accounts/test-fixtures'
import type { AssistantEvent, AssistantEventKind, ConfirmationRequest, ConversationsChanged, RunSnapshot } from '@/generated/accounts'
import { turnFragments } from './chat-model'
import { useAssistantChat } from './use-assistant-chat'

const snapshot = (over: Partial<RunSnapshot> = {}): RunSnapshot => ({
  runId: 'r1', kind: 'chat', state: 'running', text: '', tools: [], reasoning: [], toolsAvailable: true, error: null, finishedReason: null, completedAt: null, seq: 1, conversationId: 'c1', pending: null, ...over,
})
const event = (seq: number, kind: AssistantEventKind, runId = 'r1'): AssistantEvent => ({ runId, seq, kind })
const SEND: ConfirmationRequest = {
  action: 'send', tool: 'outbox_send',
  details: { type: 'send', accountName: '工作', accountAddress: 'me@example.com', to: [{ name: null, address: 'a@example.com' }], cc: [], bcc: [], subject: '你好', bodyPreview: '正文', attachments: [], warnings: [] },
}

function setup() {
  const client = testClient()
  let send: (event: AssistantEvent) => void = () => undefined
  client.onAssistantEvent.mockImplementation((listener) => { send = listener; return () => undefined })
  let changed: (change: ConversationsChanged) => void = () => undefined
  client.onAssistantConversationsChanged.mockImplementation((listener) => { changed = listener; return () => undefined })
  client.assistantStart.mockResolvedValue(snapshot())
  const emit = (...events: AssistantEvent[]) => act(() => { for (const next of events) send(next) })
  const conversations = (change: ConversationsChanged) => act(() => { changed(change) })
  const view = renderHook(() => useAssistantChat(client))
  return { client, emit, conversations, ...view }
}

afterEach(() => { vi.restoreAllMocks() })

describe('useAssistantChat', () => {
  it('sends a chat run and keeps the user message and the reply in order', async () => {
    const { client, result, emit } = setup()
    await act(async () => { await result.current.send('  你好  ') })
    expect(client.assistantStart).toHaveBeenCalledWith({ kind: { type: 'chat', message: '你好' } })
    expect(result.current.busy).toBe(true)
    await emit(event(2, { type: 'text', delta: '你好，' }), event(3, { type: 'text', delta: '有什么可以帮你？' }), event(4, { type: 'finished', reason: 'stop' }), event(5, { type: 'state', state: 'completed' }))
    await waitFor(() => expect(result.current.busy).toBe(false))
    expect(result.current.messages.map((message) => message.role)).toEqual(['user', 'assistant'])
    const reply = result.current.messages[1]
    expect(reply.role === 'assistant' && reply.turn).toMatchObject({ state: 'completed', text: '你好，有什么可以帮你？' })
  })

  it('sends what the user looks at with the message, and asks again about the same thing on retry', async () => {
    const { client, result, emit } = setup()
    const context = { type: 'email' as const, emailId: 'e1' }
    await act(async () => { await result.current.send('这封说了什么', context) })
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', message: '这封说了什么', context } })
    await emit(event(2, { type: 'state', state: 'failed' }))
    await waitFor(() => expect(result.current.busy).toBe(false))
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r2' }))
    await act(async () => { await result.current.retry() })
    // The message reached the host: it is answered again, not said twice.
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', conversationId: 'c1', message: '', regenerate: true, context } })
  })

  it('ignores an empty message and one sent while a reply is going', async () => {
    const { client, result } = setup()
    await act(async () => { expect(await result.current.send('   ')).toBe('empty') })
    await act(async () => { await result.current.send('一') })
    await act(async () => { expect(await result.current.send('二')).toBe('busy') })
    expect(client.assistantStart).toHaveBeenCalledTimes(1)
  })

  it('continues the conversation with the id the first answer brought', async () => {
    const { client, result, emit } = setup()
    await act(async () => { await result.current.send('第一句') })
    await emit(event(2, { type: 'finished', reason: 'stop' }), event(3, { type: 'state', state: 'completed' }))
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r2' }))
    await act(async () => { await result.current.send('第二句') })
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', conversationId: 'c1', message: '第二句' } })
    expect(result.current.messages.map((message) => message.role)).toEqual(['user', 'assistant', 'user', 'assistant'])
  })

  it('starts a new conversation after the one on screen was deleted or cleared elsewhere', async () => {
    const { client, result, emit, conversations } = setup()
    await act(async () => { await result.current.send('一') })
    await emit(event(2, { type: 'state', state: 'completed' }))
    await waitFor(() => expect(result.current.busy).toBe(false))
    expect(result.current.conversationId).toBe('c1')
    // Another conversation going away changes nothing.
    await conversations({ conversationId: 'other', change: 'deleted' })
    expect(result.current.conversationId).toBe('c1')
    await conversations({ conversationId: null, change: 'cleared' })
    expect(result.current.conversationId).toBeNull()
    // What was said stays on screen; the next message no longer names the cleared conversation.
    expect(result.current.messages.length).toBeGreaterThan(0)
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r2', conversationId: 'c2' }))
    await act(async () => { await result.current.send('二') })
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', message: '二' } })
  })

  it('stops the reply, and keeps the words written so far', async () => {
    const { client, result, emit } = setup()
    await act(async () => { await result.current.send('写点什么') })
    await emit(event(2, { type: 'text', delta: '写到一半' }))
    act(() => result.current.stop())
    expect(client.assistantCancel).toHaveBeenCalledWith('r1')
    await emit(event(3, { type: 'state', state: 'canceled' }))
    const reply = result.current.messages[1]
    expect(reply.role === 'assistant' && reply.turn).toMatchObject({ state: 'canceled', text: '写到一半' })
  })

  it('answers the confirmation the reply waits on, and remembers the answer', async () => {
    const { client, result, emit } = setup()
    await act(async () => { await result.current.send('发出去') })
    await emit(
      event(2, { type: 'text', delta: '好的。' }),
      event(3, { type: 'tool', callId: 'k1', name: 'outbox_send', status: 'running', summary: { type: 'pending' } }),
      event(4, { type: 'state', state: 'awaitingConfirmation' }),
      event(5, { type: 'confirm', requestId: 'q1', callId: 'k1', request: SEND }),
    )
    expect(result.current.pending).toMatchObject({ requestId: 'q1' })
    await act(async () => { await result.current.respond(true) })
    expect(client.assistantRespond).toHaveBeenCalledWith('r1', 'q1', true)
    await emit(event(6, { type: 'state', state: 'running' }))
    expect(result.current.pending).toBeNull()
    const reply = result.current.messages[1]
    expect(reply.role === 'assistant' && reply.turn.answers).toEqual({ q1: true })
  })

  it('forgets an answer the host did not accept', async () => {
    const { client, result, emit } = setup()
    await act(async () => { await result.current.send('发出去') })
    await emit(event(2, { type: 'state', state: 'awaitingConfirmation' }), event(3, { type: 'confirm', requestId: 'q1', callId: 'k1', request: SEND }))
    client.assistantRespond.mockRejectedValueOnce(new Error('conflict'))
    await act(async () => { await result.current.respond(false) })
    const reply = result.current.messages[1]
    expect(reply.role === 'assistant' && reply.turn.answers).toEqual({})
  })

  it('retries a failed reply with the same message and without a second copy of it', async () => {
    const { client, result, emit } = setup()
    await act(async () => { await result.current.send('帮我看看') })
    await emit(event(2, { type: 'failed', code: 'aiNetwork' }), event(3, { type: 'state', state: 'failed' }))
    expect(result.current.messages[1]).toMatchObject({ role: 'assistant', turn: { state: 'failed', error: 'aiNetwork' } })
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r2' }))
    await act(async () => { await result.current.retry() })
    // The host has the message already: the reply is asked for again, the message is not added twice.
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', conversationId: 'c1', message: '', regenerate: true } })
    expect(result.current.messages.map((message) => message.role)).toEqual(['user', 'assistant'])
    expect(result.current.messages[1]).toMatchObject({ turn: { state: 'running', error: null } })
  })

  it('shows a refused start as a failed reply that can be retried', async () => {
    const { client, result } = setup()
    client.assistantStart.mockRejectedValueOnce(new Error('boom'))
    await act(async () => { expect(await result.current.send('你好')).toBe('failed') })
    expect(result.current.messages[1]).toMatchObject({ role: 'assistant', turn: { state: 'failed', error: 'internal' } })
  })

  it('leaves no trace of a message that waits for consent', async () => {
    const { client, result } = setup()
    client.assistantStart.mockResolvedValueOnce(snapshot({ state: 'awaitingConsent', conversationId: 'never' }))
    await act(async () => { expect(await result.current.send('你好')).toBe('consent') })
    expect(result.current.messages).toEqual([])
    // The conversation id of that snapshot is not adopted.
    client.assistantStart.mockResolvedValue(snapshot())
    await act(async () => { await result.current.send('你好') })
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', message: '你好' } })
  })

  it('starts over on request', async () => {
    const { client, result } = setup()
    await act(async () => { await result.current.send('你好') })
    act(() => result.current.reset())
    expect(client.assistantCancel).toHaveBeenCalledWith('r1')
    expect(result.current.messages).toEqual([])
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r2', conversationId: 'c2' }))
    await act(async () => { await result.current.send('新的') })
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', message: '新的' } })
  })

  it('learns that the model has no tools when a reply ends', async () => {
    const { client, result, emit } = setup()
    expect(result.current.toolsAvailable).toBe(true)
    await act(async () => { await result.current.send('你好') })
    client.assistantSnapshot.mockResolvedValue(snapshot({ state: 'completed', toolsAvailable: false }))
    await emit(event(2, { type: 'finished', reason: 'stop' }), event(3, { type: 'state', state: 'completed' }))
    await waitFor(() => expect(result.current.toolsAvailable).toBe(false))
  })
})

describe('asking again', () => {
  async function talked() {
    const view = setup()
    const { client, result, emit } = view
    await act(async () => { await result.current.send('第一句') })
    await emit(event(2, { type: 'text', delta: '答一' }), event(3, { type: 'finished', reason: 'stop' }), event(4, { type: 'state', state: 'completed' }))
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r2' }))
    await act(async () => { await result.current.send('第二句') })
    await emit(event(2, { type: 'text', delta: '答二' }, 'r2'), event(3, { type: 'finished', reason: 'stop' }, 'r2'), event(4, { type: 'state', state: 'completed' }, 'r2'))
    await waitFor(() => expect(result.current.busy).toBe(false))
    return view
  }
  const said = (messages: ReturnType<typeof useAssistantChat>['messages']) =>
    messages.map((message) => (message.role === 'user' ? message.text : message.turn.text))

  it('rewords an earlier message in place and drops everything after it', async () => {
    const { client, result, emit } = await talked()
    const first = result.current.messages[0]
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r3' }))
    await act(async () => { expect(await result.current.edit(first.id, '第一句，改过')).toBe('sent') })
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', conversationId: 'c1', message: '第一句，改过', edit: 0 } })
    await emit(event(2, { type: 'text', delta: '新答' }, 'r3'), event(3, { type: 'finished', reason: 'stop' }, 'r3'), event(4, { type: 'state', state: 'completed' }, 'r3'))
    await waitFor(() => expect(said(result.current.messages)).toEqual(['第一句，改过', '新答']))
  })

  it('puts everything back when the edit does not go out', async () => {
    const { client, result } = await talked()
    const before = said(result.current.messages)
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r3', state: 'awaitingConsent' }))
    await act(async () => { expect(await result.current.edit(result.current.messages[2].id, '改')).toBe('consent') })
    expect(said(result.current.messages)).toEqual(before)
  })

  it('replaces the last reply, not the one before it, when it is gone over', async () => {
    const { client, result, emit } = await talked()
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r3' }))
    await act(async () => { await result.current.regenerate() })
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', conversationId: 'c1', message: '', regenerate: true } })
    expect(said(result.current.messages)).toEqual(['第一句', '答一', '第二句', ''])
    await emit(event(2, { type: 'text', delta: '再答二' }, 'r3'), event(3, { type: 'state', state: 'completed' }, 'r3'))
    await waitFor(() => expect(said(result.current.messages)).toEqual(['第一句', '答一', '第二句', '再答二']))
  })

  it('shows a reply that was gone on with as one reply', async () => {
    const { client, result, emit } = setup()
    await act(async () => { await result.current.send('写长一点') })
    await emit(event(2, { type: 'text', delta: '前半' }), event(3, { type: 'state', state: 'canceled' }))
    await waitFor(() => expect(result.current.busy).toBe(false))
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r2' }))
    await act(async () => { await result.current.continueReply() })
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: { type: 'chat', conversationId: 'c1', message: '', continuation: true } })
    await emit(event(2, { type: 'text', delta: '后半' }, 'r2'), event(3, { type: 'state', state: 'completed' }, 'r2'))
    await waitFor(() => expect(said(result.current.messages)).toEqual(['写长一点', '前半后半']))
  })
})

describe('turnFragments', () => {
  it('orders text and tools by where each began, and hides the row of a confirmed call', async () => {
    const { result, emit } = setup()
    await act(async () => { await result.current.send('x') })
    await emit(
      event(2, { type: 'text', delta: '先看看。' }),
      event(3, { type: 'tool', callId: 'k1', name: 'mail_search', status: 'done', summary: { type: 'found', count: 2 } }),
      event(4, { type: 'text', delta: '找到了。' }),
      event(5, { type: 'tool', callId: 'k2', name: 'outbox_send', status: 'running', summary: { type: 'pending' } }),
      event(6, { type: 'state', state: 'awaitingConfirmation' }),
      event(7, { type: 'confirm', requestId: 'q1', callId: 'k2', request: SEND }),
    )
    const reply = result.current.messages[1]
    if (reply.role !== 'assistant') throw new Error('expected a reply')
    expect(turnFragments(reply.turn).map((fragment) => (fragment.kind === 'text' ? `text:${fragment.text}` : fragment.kind))).toEqual(['text:先看看。', 'tool', 'text:找到了。', 'confirm'])
  })
})
