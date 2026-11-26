import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { ChatContext, PendingConfirmation } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { joinTurns, savedMessages, type ChatMessage, type ChatTurn } from './chat-model'
import { useAssistantRun, type AssistantRun } from './use-assistant-run'

/** How a `send` ended: `consent` means nothing was sent and the user must agree first. */
export type SendResult = 'sent' | 'consent' | 'failed' | 'busy' | 'empty'

/** How opening a saved conversation ended: `stale` means a newer request (or a reset) took its place. */
export type OpenResult = 'opened' | 'failed' | 'busy' | 'stale'

export type AssistantChat = {
  /** Everything said in this conversation, oldest first; the last assistant turn may still be streaming. */
  messages: ChatMessage[]
  /** A reply is on its way (starting, running or waiting for a confirmation). */
  busy: boolean
  /** The confirmation the reply waits on. */
  pending: PendingConfirmation | null
  /** False once the host said the model cannot use tools; true until then. */
  toolsAvailable: boolean
  /** `context` is what the user is looking at (an email, a selection, a draft); the core reads it for this reply only. */
  send: (message: string, context?: ChatContext | null) => Promise<SendResult>
  /** Stops the reply; what was written stays. */
  stop: () => void
  /** Answers the pending confirmation. */
  respond: (approved: boolean) => Promise<void>
  /** Asks again after a failure: a new reply in place of the failed one. */
  retry: () => Promise<SendResult>
  /** Answers the last message again in place of its reply; refused when that reply changed mail. */
  regenerate: () => Promise<SendResult>
  /** Goes on with a reply that stopped short (stopped, or cut off by a limit); shown as one reply. */
  continueReply: () => Promise<SendResult>
  /**
   * Rewords an earlier message of yours: it and everything after it are replaced, and the reworded
   * message is answered. Refused (`failed`) when a reply after it changed mail.
   */
  edit: (messageId: string, text: string, context?: ChatContext | null) => Promise<SendResult>
  /** Starts over with an empty conversation. */
  reset: () => void
  /** The saved conversation this one belongs to; null until the first answer of a new conversation brings its id. */
  conversationId: string | null
  /** Changes whenever another conversation takes the screen (opened from history, or a new one), so a view can fade and rewind. */
  viewKey: number
  /**
   * Shows a saved conversation and makes it the one the next message continues. A reply still going in
   * the conversation being left is stopped (the core cancels the run; what it wrote so far is saved).
   */
  openConversation: (id: string) => Promise<OpenResult>
  /** Calls whose undo was used, so a remounted page does not offer it again. */
  undone: ReadonlySet<string>
  markUndone: (callId: string) => void
}

const turnOf = (run: AssistantRun, answers: Record<string, boolean>): ChatTurn => ({
  state: run.state, text: run.text, tools: run.tools, reasoning: run.reasoning, confirms: run.confirms, answers,
  pendingId: run.pending?.requestId ?? null, error: run.error, finishedReason: run.finishedReason, finishedAt: run.finishedAt,
})

/**
 * One conversation with the assistant, kept in memory for this app session (ai-chat-ui.md 3, data
 * layer). It wraps the run hook: each `send` starts a chat run with the conversation id the first
 * answer brought back; the reply being written is the last message, and a finished one is set aside
 * when the next message goes out. Only one reply runs at a time. A reply still going when the owner
 * unmounts is cancelled (the run hook does), so keep the owner mounted for as long as the
 * conversation should live.
 */
export function useAssistantChat(client: MailxClient): AssistantChat {
  const run = useAssistantRun(client)
  const [history, setHistory] = useState<ChatMessage[]>([])
  const [open, setOpen] = useState(false)
  const [answers, setAnswers] = useState<Record<string, boolean>>({})
  const [undone, setUndone] = useState<ReadonlySet<string>>(() => new Set())
  const [noTools, setNoTools] = useState(false)
  const conversation = useRef<string | null>(null)
  const [conversationId, setConversationId] = useState<string | null>(null)
  const [viewKey, setViewKey] = useState(0)
  const opening = useRef(0)
  const lastMessage = useRef<string | null>(null)
  const lastContext = useRef<ChatContext | null>(null)
  const counter = useRef(0)
  const sending = useRef(false)
  const [turnNo, setTurnNo] = useState(0)
  /** The reply the live run goes on with, shown before it as one reply. */
  const [carried, setCarried] = useState<ChatTurn | null>(null)
  /** The last message reached the host (its run started), so asking again needs no new message. */
  const started = useRef(false)

  const live = useMemo(
    () => {
      if (!open) return null
      const turn = turnOf(run, answers)
      return carried ? joinTurns(carried, turn) : turn
    },
    // The run object is new every render; its parts are what changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [open, carried, run.state, run.text, run.tools, run.reasoning, run.confirms, run.pending, run.error, run.finishedReason, run.finishedAt, answers],
  )
  const latest = useRef({ run, live, open })
  useEffect(() => { latest.current = { run, live, open } })

  // The conversation on screen was deleted elsewhere (the history panel of another window,
  // Settings → clear all, an account removal): keep showing it, but the next message starts a
  // new conversation instead of continuing one the host no longer has.
  useEffect(() => client.onAssistantConversationsChanged((change) => {
    const gone = change.change === 'cleared' || (change.change === 'deleted' && change.conversationId === conversation.current)
    if (gone && conversation.current) {
      conversation.current = null
      setConversationId(null)
    }
  }), [client])

  const begin = useCallback(async (message: string, context: ChatContext | null): Promise<SendResult> => {
    const snapshot = await latest.current.run.start({
      type: 'chat',
      ...(conversation.current ? { conversationId: conversation.current } : {}),
      message,
      ...(context ? { context } : {}),
    })
    if (snapshot?.state === 'awaitingConsent') return 'consent'
    if (!snapshot) return 'failed'
    // The conversation exists in the host only once a run really started.
    conversation.current = snapshot.conversationId ?? conversation.current
    setConversationId(conversation.current)
    return 'sent'
  }, [])

  const send = useCallback(async (text: string, context: ChatContext | null = null): Promise<SendResult> => {
    const message = text.trim()
    if (message === '') return 'empty'
    // A run left over from a conversation that was cleared does not count: starting replaces it.
    if ((latest.current.run.active && latest.current.open) || sending.current) return 'busy'
    sending.current = true
    counter.current += 1
    setTurnNo(counter.current)
    const userId = `user-${counter.current}`
    const { live: previous, open: wasOpen } = latest.current
    const settled: ChatMessage[] = wasOpen && previous ? [{ id: `reply-${counter.current - 1}`, role: 'assistant', turn: previous }] : []
    setHistory((list) => [...list, ...settled, { id: userId, role: 'user', text: message, at: Date.now() }])
    setCarried(null)
    setOpen(true)
    lastMessage.current = message
    lastContext.current = context
    const result = await begin(message, context).finally(() => { sending.current = false })
    started.current = result === 'sent'
    if (result === 'consent') {
      // Nothing went out: the message is not part of the conversation yet.
      setHistory((list) => list.filter((entry) => entry.id !== userId))
      setOpen(false)
      lastMessage.current = null
      lastContext.current = null
    }
    return result
  }, [begin])

  /** What is on screen, live reply included: what asking again starts from and goes back to. */
  const shown = useRef<ChatMessage[]>([])

  /**
   * Starts a run in place of earlier history: the view becomes `next` at once, and goes back to what
   * it was if the run does not start (no consent, refused, unreachable); the host keeps its history then too.
   */
  const redo = useCallback(async (
    kind: ({ regenerate: true } | { continuation: true } | { edit: number; message: string }) & { context?: ChatContext },
    next: { history: ChatMessage[]; carried: ChatTurn | null },
  ): Promise<SendResult> => {
    const id = conversation.current
    if (!id || (latest.current.run.active && latest.current.open) || sending.current) return 'busy'
    sending.current = true
    const before = shown.current
    counter.current += 1
    setTurnNo(counter.current)
    setHistory(next.history)
    setCarried(next.carried)
    setOpen(true)
    try {
      const snapshot = await latest.current.run.start({ type: 'chat', conversationId: id, message: '', ...kind })
      const result: SendResult = !snapshot || snapshot.state === 'failed' ? 'failed' : snapshot.state === 'awaitingConsent' ? 'consent' : 'sent'
      if (result === 'sent') started.current = true
      else {
        setHistory(before)
        setCarried(null)
        setOpen(false)
      }
      return result
    } finally {
      sending.current = false
    }
  }, [])

  const regenerate = useCallback((): Promise<SendResult> => {
    const list = shown.current
    const history = list.at(-1)?.role === 'assistant' ? list.slice(0, -1) : list
    // About what the last message was about (the view is for one run only).
    const context = lastContext.current
    return redo({ regenerate: true, ...(context ? { context } : {}) }, { history, carried: null })
  }, [redo])

  const continueReply = useCallback(async (): Promise<SendResult> => {
    const list = shown.current
    const last = list.at(-1)
    if (last?.role !== 'assistant') return 'failed'
    const context = lastContext.current
    return redo({ continuation: true, ...(context ? { context } : {}) }, { history: list.slice(0, -1), carried: last.turn })
  }, [redo])

  const edit = useCallback(async (messageId: string, text: string, context: ChatContext | null = null): Promise<SendResult> => {
    const message = text.trim()
    if (message === '') return 'empty'
    const list = shown.current
    const at = list.findIndex((entry) => entry.id === messageId)
    if (at < 0 || list[at].role !== 'user') return 'failed'
    const index = list.slice(0, at).filter((entry) => entry.role === 'user').length
    const history: ChatMessage[] = [...list.slice(0, at), { id: `user-${counter.current + 1}`, role: 'user', text: message, at: Date.now() }]
    const result = await redo({ edit: index, message, ...(context ? { context } : {}) }, { history, carried: null })
    if (result === 'sent') {
      lastMessage.current = message
      lastContext.current = context
    }
    return result
  }, [redo])

  const retry = useCallback(async (): Promise<SendResult> => {
    // The failed message is in the host's history already: answer it again rather than say it twice.
    if (started.current && conversation.current) {
      const result = await regenerate()
      return result === 'consent' ? 'failed' : result
    }
    const message = lastMessage.current
    if (!message || (latest.current.run.active && latest.current.open) || sending.current) return 'busy'
    sending.current = true
    // Asked again about what it was asked about then, even if the screen moved on.
    const result = await begin(message, lastContext.current).finally(() => { sending.current = false })
    started.current = result === 'sent'
    return result === 'consent' ? 'failed' : result
  }, [begin, regenerate])

  const stop = useCallback(() => latest.current.run.cancel(), [])

  const respond = useCallback(async (approved: boolean) => {
    const { run: current } = latest.current
    const pending = current.pending
    if (!pending || !current.runId) return
    setAnswers((map) => ({ ...map, [pending.requestId]: approved }))
    try {
      await client.assistantRespond(current.runId, pending.requestId, approved)
    } catch {
      // The request is gone (answered elsewhere, timed out, run over): the run's own events say what became of it.
      setAnswers((map) => Object.fromEntries(Object.entries(map).filter(([id]) => id !== pending.requestId)))
    }
  }, [client])

  const reset = useCallback(() => {
    latest.current.run.cancel()
    opening.current += 1
    conversation.current = null
    setConversationId(null)
    setViewKey((key) => key + 1)
    lastMessage.current = null
    lastContext.current = null
    started.current = false
    setHistory([])
    setCarried(null)
    setOpen(false)
    setAnswers({})
    setUndone(new Set())
  }, [])

  const openConversation = useCallback(async (id: string): Promise<OpenResult> => {
    if (id === conversation.current) return 'opened'
    if (sending.current) return 'busy'
    opening.current += 1
    const ticket = opening.current
    let detail
    try {
      detail = await client.assistantGetConversation(id)
    } catch {
      return ticket === opening.current ? 'failed' : 'stale'
    }
    if (ticket !== opening.current) return 'stale'
    if (sending.current) return 'busy'
    // Leaving a conversation with a reply still being written stops that reply; nothing is asked.
    latest.current.run.cancel()
    conversation.current = id
    setConversationId(id)
    lastMessage.current = null
    lastContext.current = null
    started.current = false
    setHistory(savedMessages(detail))
    setCarried(null)
    setOpen(false)
    setAnswers({})
    setUndone(new Set())
    setNoTools(false)
    setViewKey((key) => key + 1)
    return 'opened'
  }, [client])

  const markUndone = useCallback((callId: string) => setUndone((set) => new Set(set).add(callId)), [])

  // When a reply ends, ask whether the model could use tools: the host learns that mid-run, after the first snapshot.
  const { state, runId, toolsAvailable } = run
  useEffect(() => {
    if (!open || !runId || (state !== 'completed' && state !== 'failed')) return
    let current = true
    Promise.resolve(toolsAvailable ? client.assistantSnapshot(runId) : undefined).then((snapshot) => {
      if (current) setNoTools(snapshot ? !snapshot.toolsAvailable : !toolsAvailable)
    }, () => undefined)
    return () => { current = false }
  }, [client, open, runId, state, toolsAvailable])

  const messages = useMemo<ChatMessage[]>(
    () => (live ? [...history, { id: `reply-${turnNo}`, role: 'assistant', turn: live }] : history),
    [history, live, turnNo],
  )
  useEffect(() => { shown.current = messages })
  return {
    messages, busy: open && run.active, pending: open ? run.pending : null, toolsAvailable: !noTools,
    send, stop, respond, retry, regenerate, continueReply, edit, reset, conversationId, viewKey, openConversation, undone, markUndone,
  }
}
