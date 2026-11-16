import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { PendingConfirmation } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import type { ChatMessage, ChatTurn } from './chat-model'
import { useAssistantRun, type AssistantRun } from './use-assistant-run'

/** How a `send` ended: `consent` means nothing was sent and the user must agree first. */
export type SendResult = 'sent' | 'consent' | 'failed' | 'busy' | 'empty'

export type AssistantChat = {
  /** Everything said in this conversation, oldest first; the last assistant turn may still be streaming. */
  messages: ChatMessage[]
  /** A reply is on its way (starting, running or waiting for a confirmation). */
  busy: boolean
  /** The confirmation the reply waits on. */
  pending: PendingConfirmation | null
  /** False once the host said the model cannot use tools; true until then. */
  toolsAvailable: boolean
  send: (message: string) => Promise<SendResult>
  /** Stops the reply; what was written stays. */
  stop: () => void
  /** Answers the pending confirmation. */
  respond: (approved: boolean) => Promise<void>
  /** Asks again after a failure: the same message, a new reply in place of the failed one. */
  retry: () => Promise<SendResult>
  /** Starts over with an empty conversation. */
  reset: () => void
  /** Calls whose undo was used, so a remounted page does not offer it again. */
  undone: ReadonlySet<string>
  markUndone: (callId: string) => void
}

const turnOf = (run: AssistantRun, answers: Record<string, boolean>): ChatTurn => ({
  state: run.state, text: run.text, tools: run.tools, confirms: run.confirms, answers,
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
  const lastMessage = useRef<string | null>(null)
  const counter = useRef(0)
  const sending = useRef(false)
  const [turnNo, setTurnNo] = useState(0)

  const live = useMemo(
    () => (open ? turnOf(run, answers) : null),
    // The run object is new every render; its parts are what changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [open, run.state, run.text, run.tools, run.confirms, run.pending, run.error, run.finishedReason, run.finishedAt, answers],
  )
  const latest = useRef({ run, live, open })
  useEffect(() => { latest.current = { run, live, open } })

  const begin = useCallback(async (message: string): Promise<SendResult> => {
    const snapshot = await latest.current.run.start({ type: 'chat', ...(conversation.current ? { conversationId: conversation.current } : {}), message })
    if (snapshot?.state === 'awaitingConsent') return 'consent'
    if (!snapshot) return 'failed'
    // The conversation exists in the host only once a run really started.
    conversation.current = snapshot.conversationId ?? conversation.current
    return 'sent'
  }, [])

  const send = useCallback(async (text: string): Promise<SendResult> => {
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
    setHistory((list) => [...list, ...settled, { id: userId, role: 'user', text: message }])
    setOpen(true)
    lastMessage.current = message
    const result = await begin(message).finally(() => { sending.current = false })
    if (result === 'consent') {
      // Nothing went out: the message is not part of the conversation yet.
      setHistory((list) => list.filter((entry) => entry.id !== userId))
      setOpen(false)
      lastMessage.current = null
    }
    return result
  }, [begin])

  const retry = useCallback(async (): Promise<SendResult> => {
    const message = lastMessage.current
    if (!message || (latest.current.run.active && latest.current.open) || sending.current) return 'busy'
    sending.current = true
    const result = await begin(message).finally(() => { sending.current = false })
    return result === 'consent' ? 'failed' : result
  }, [begin])

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
    conversation.current = null
    lastMessage.current = null
    setHistory([])
    setOpen(false)
    setAnswers({})
    setUndone(new Set())
  }, [])

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
  return { messages, busy: open && run.active, pending: open ? run.pending : null, toolsAvailable: !noTools, send, stop, respond, retry, reset, undone, markUndone }
}
