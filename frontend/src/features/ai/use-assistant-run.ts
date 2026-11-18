import { useCallback, useEffect, useRef, useState } from 'react'
import type { AssistantEvent, ConfirmationRequest, ErrorCode, FinishedReason, PendingConfirmation, RunKind, RunSnapshot, RunState, ToolActivity } from '@/generated/accounts'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'

/** `idle`: nothing started yet. `starting`: waiting for the host to answer `assistantStart`. */
export type AssistantRunState = 'idle' | 'starting' | RunState

/** A tool call with where in the text it began, so a view can show the reply's parts in the order they came. */
/** `settledAt` is when this window first saw the call finish (a clock for undo windows), null while it runs. */
export type RunTool = ToolActivity & { at: number; settledAt: number | null }
/**
 * One stretch of the model's thinking. `at` (text offset) and `toolsBefore` (tools that began before it)
 * place it among the reply's parts; `startedAt` is a `Date.now()` time; `durationMs` is null while it goes.
 */
export type RunThought = { segment: number; text: string; at: number; toolsBefore: number; startedAt: number; durationMs: number | null }
/** A confirmation the run asked for, kept after it is answered; `at` as for `RunTool`. */
export type RunConfirm = { requestId: string; callId: string; request: ConfirmationRequest; at: number }

export type AssistantRun = {
  runId: string | null
  state: AssistantRunState
  /** Everything written so far. Committed at most once per animation frame. */
  text: string
  error: ErrorCode | null
  finishedReason: FinishedReason | null
  /** When this window saw the run complete (the host sends no time of its own). */
  finishedAt: Date | null
  /** Tool calls in the order they began; the latest state of each. Chat runs only. */
  tools: RunTool[]
  /** The model's thinking, stretch by stretch. Its text is committed at most once per frame, like `text`. */
  reasoning: RunThought[]
  /** Every confirmation this run asked for, in order. */
  confirms: RunConfirm[]
  /** The confirmation the run is waiting on now. */
  pending: PendingConfirmation | null
  /** The conversation a chat run belongs to. */
  conversationId: string | null
  /** False once the host says the model cannot call tools. Absent means yes. */
  toolsAvailable: boolean
  /** Starting, running or waiting for a confirmation. */
  active: boolean
  /**
   * Starts a run, cancelling the one that is still going. Resolves with the first snapshot, or `null`
   * when the host refused or a newer start (or leaving) replaced this one.
   */
  start: (kind: RunKind) => Promise<RunSnapshot | null>
  /** Cancels the current run; its text so far stays. */
  cancel: () => void
}

type View = {
  runId: string | null; state: AssistantRunState; text: string; error: ErrorCode | null; finishedReason: FinishedReason | null; finishedAt: Date | null
  tools: RunTool[]; reasoning: RunThought[]; confirms: RunConfirm[]; pending: PendingConfirmation | null; conversationId: string | null; toolsAvailable: boolean
}

const IDLE: View = {
  runId: null, state: 'idle', text: '', error: null, finishedReason: null, finishedAt: null,
  tools: [], reasoning: [], confirms: [], pending: null, conversationId: null, toolsAvailable: true,
}
const ENDED: ReadonlySet<AssistantRunState> = new Set(['completed', 'failed', 'canceled', 'awaitingConsent'])
/** Events seen before `assistantStart` answered are kept briefly, so none is lost to the race. */
const EARLY_LIMIT = 200

export const runIsActive = (state: AssistantRunState) => state === 'starting' || state === 'running' || state === 'awaitingConfirmation'

/** Host-side codes (`hostUnavailable`, `transport`) are not core error codes; they read as internal. */
/** Stretches still open when the run ends lasted until now. */
const closeThoughts = (thoughts: RunThought[]) => thoughts.some((thought) => thought.durationMs === null)
  ? thoughts.map((thought) => thought.durationMs === null ? { ...thought, durationMs: Math.max(0, Date.now() - thought.startedAt) } : thought)
  : thoughts

const failureCode = (error: unknown): ErrorCode => {
  const code = error instanceof MailxClientError ? error.failure.code : null
  return code && code !== 'hostUnavailable' && code !== 'transport' ? code : 'internal'
}

/**
 * Follows one assistant run at a time (ai-chat-ui.md, data layer). The host sends events with a
 * per-run `seq`; this applies them in order, ignores ones it already has, and asks for a snapshot
 * when one is missing or after a remount. Text deltas are buffered and committed once per frame.
 * A run still going when the owner unmounts is cancelled. `resume` names a run to pick up after a
 * remount: its first snapshot restores the state and later events continue from it.
 */
export function useAssistantRun(client: MailxClient, options: { resume?: string } = {}): AssistantRun {
  const [view, setView] = useState<View>(IDLE)
  const live = useRef<View>(IDLE)
  const runId = useRef<string | null>(null)
  const lastSeq = useRef(0)
  const generation = useRef(0)
  const starting = useRef(false)
  const early = useRef<AssistantEvent[]>([])
  const recovering = useRef(false)
  const held = useRef<AssistantEvent[]>([])
  const frame = useRef<number | null>(null)
  const mounted = useRef(true)

  const commit = useCallback(() => {
    if (frame.current !== null) { cancelAnimationFrame(frame.current); frame.current = null }
    setView(live.current)
  }, [])
  const schedule = useCallback(() => {
    if (frame.current !== null) return
    frame.current = requestAnimationFrame(() => { frame.current = null; setView(live.current) })
  }, [])
  /** Text goes through the frame; anything else shows at once, text included, so the order holds. */
  const update = useCallback((change: Partial<View>, immediate: boolean) => {
    live.current = { ...live.current, ...change }
    if (immediate) commit()
    else schedule()
  }, [commit, schedule])

  const adopt = useCallback((snapshot: RunSnapshot) => {
    lastSeq.current = snapshot.seq
    // The host's time wins (a cached answer was generated earlier); this window's clock is the fallback.
    const hostTime = snapshot.completedAt ? new Date(snapshot.completedAt) : null
    const finishedAt = snapshot.state === 'completed'
      ? (hostTime && !Number.isNaN(hostTime.getTime()) ? hostTime : live.current.finishedAt ?? new Date())
      : null
    // The snapshot has no order between text and tools: what this window already placed keeps its place, the rest goes last.
    const known = new Map(live.current.tools.map((tool) => [tool.callId, tool.at]))
    const seen = new Map(live.current.tools.map((tool) => [tool.callId, tool.settledAt]))
    const tools = snapshot.tools.map((tool): RunTool => ({
      ...tool,
      at: known.get(tool.callId) ?? snapshot.text.length,
      settledAt: tool.status === 'running' ? null : seen.get(tool.callId) ?? Date.now(),
    }))
    // This window's start time wins for a stretch it already follows, so its clock does not jump.
    const started = new Map(live.current.reasoning.map((thought) => [thought.segment, thought.startedAt]))
    const reasoning = snapshot.reasoning.map((thought, segment): RunThought => {
      const hostStart = Date.parse(thought.startedAt)
      return {
        segment, text: thought.text, at: thought.textOffset, toolsBefore: thought.toolsBefore, durationMs: thought.durationMs,
        startedAt: started.get(segment) ?? (Number.isNaN(hostStart) ? Date.now() : hostStart),
      }
    })
    const confirms = live.current.confirms.slice()
    if (snapshot.pending && !confirms.some((item) => item.requestId === snapshot.pending!.requestId)) confirms.push({ ...snapshot.pending, at: snapshot.text.length })
    update({
      runId: snapshot.runId, state: snapshot.state, text: snapshot.text, error: snapshot.error, finishedReason: snapshot.finishedReason, finishedAt,
      tools, reasoning: ENDED.has(snapshot.state) ? closeThoughts(reasoning) : reasoning, confirms, pending: snapshot.pending, conversationId: snapshot.conversationId ?? live.current.conversationId,
      toolsAvailable: snapshot.toolsAvailable,
    }, true)
  }, [update])

  const apply = useCallback((event: AssistantEvent) => {
    lastSeq.current = event.seq
    const { kind } = event
    switch (kind.type) {
      case 'text': update({ text: live.current.text + kind.delta }, false); break
      case 'reasoning': {
        const { reasoning } = live.current
        const open = reasoning.find((thought) => thought.segment === kind.segment)
        if (open) {
          // More words for a stretch already on screen: they wait for the frame, like text.
          if (kind.delta) update({ reasoning: reasoning.map((thought) => thought === open ? { ...thought, text: thought.text + kind.delta } : thought) }, false)
        } else {
          const next: RunThought = {
            segment: kind.segment, text: kind.delta, at: live.current.text.length, toolsBefore: live.current.tools.length, startedAt: Date.now(), durationMs: null,
          }
          update({ reasoning: [...reasoning, next] }, true)
        }
        break
      }
      case 'reasoningEnd': update({
        reasoning: live.current.reasoning.map((thought) => thought.segment === kind.segment ? { ...thought, durationMs: kind.durationMs } : thought),
      }, true); break
      case 'state': update({
        state: kind.state,
        reasoning: ENDED.has(kind.state) ? closeThoughts(live.current.reasoning) : live.current.reasoning,
        pending: kind.state === 'awaitingConfirmation' ? live.current.pending : null,
        finishedAt: kind.state === 'completed' ? new Date() : live.current.finishedAt,
      }, true); break
      case 'tool': {
        const before = live.current.tools.find((tool) => tool.callId === kind.callId)
        const at = before?.at ?? live.current.text.length
        const settledAt = kind.status === 'running' ? null : before?.settledAt ?? Date.now()
        const next: RunTool = { callId: kind.callId, name: kind.name, status: kind.status, summary: kind.summary, at, settledAt }
        const tools = live.current.tools.some((tool) => tool.callId === kind.callId)
          ? live.current.tools.map((tool) => tool.callId === kind.callId ? next : tool)
          : [...live.current.tools, next]
        update({ tools }, true)
        break
      }
      case 'confirm': {
        const pending: PendingConfirmation = { requestId: kind.requestId, callId: kind.callId, request: kind.request }
        const confirms = live.current.confirms.some((item) => item.requestId === kind.requestId)
          ? live.current.confirms : [...live.current.confirms, { ...pending, at: live.current.text.length }]
        update({ pending, confirms }, true)
        break
      }
      case 'finished': update({ finishedReason: kind.reason }, true); break
      case 'failed': update({ error: kind.code }, true); break
      default: break
    }
  }, [update])

  const recover = useCallback((id: string) => {
    if (recovering.current) return
    recovering.current = true
    const mine = generation.current
    const current = () => mine === generation.current && runId.current === id
    /** One snapshot, then whatever was held past it; true when a gap is still open and another is needed. */
    function round() {
      client.assistantSnapshot(id).then((snapshot) => {
        if (!current()) return false
        adopt(snapshot)
        const queued = held.current.filter((event) => event.seq > lastSeq.current).sort((a, b) => a.seq - b.seq)
        held.current = []
        for (const event of queued) {
          if (event.seq === lastSeq.current + 1) apply(event)
          else { held.current = [event]; break }
        }
        return held.current.length > 0
      }, () => {
        // The run is gone (ten minutes after it ended) or the host is: treat it as failed.
        if (current()) update({ state: 'failed', error: 'internal' }, true)
        return false
      }).then((again) => {
        if (!current()) return
        if (again) round()
        else recovering.current = false
      })
    }
    round()
  }, [adopt, apply, client, update])

  const receive = useCallback((event: AssistantEvent) => {
    if (runId.current === null || event.runId !== runId.current) {
      if (starting.current && early.current.length < EARLY_LIMIT) early.current.push(event)
      return
    }
    if (ENDED.has(live.current.state)) return
    if (event.seq <= lastSeq.current) return
    if (recovering.current) { held.current.push(event); return }
    if (event.seq > lastSeq.current + 1) {
      held.current.push(event)
      recover(event.runId)
      return
    }
    apply(event)
  }, [apply, recover])

  useEffect(() => client.onAssistantEvent(receive), [client, receive])

  const cancelId = useCallback((id: string) => { client.assistantCancel(id).catch(() => undefined) }, [client])

  const reset = useCallback(() => {
    generation.current += 1
    runId.current = null
    lastSeq.current = 0
    early.current = []
    held.current = []
    recovering.current = false
  }, [])

  const start = useCallback(async (kind: RunKind): Promise<RunSnapshot | null> => {
    const previous = runId.current
    if (previous && runIsActive(live.current.state)) cancelId(previous)
    reset()
    const mine = generation.current
    starting.current = true
    live.current = { ...IDLE, state: 'starting' }
    commit()
    let snapshot: RunSnapshot
    try {
      snapshot = await client.assistantStart({ kind })
    } catch (error) {
      if (mine !== generation.current) return null
      starting.current = false
      update({ state: 'failed', error: failureCode(error) }, true)
      return null
    }
    if (mine !== generation.current || !mounted.current) {
      // Replaced or left while the host was answering: do not leave a run going that nobody follows.
      if (!ENDED.has(snapshot.state)) cancelId(snapshot.runId)
      return null
    }
    starting.current = false
    runId.current = snapshot.runId
    adopt(snapshot)
    const queued = early.current.filter((event) => event.runId === snapshot.runId && event.seq > snapshot.seq).sort((a, b) => a.seq - b.seq)
    early.current = []
    for (const event of queued) {
      if (event.seq === lastSeq.current + 1) apply(event)
      else if (event.seq > lastSeq.current + 1) { held.current.push(event) }
    }
    if (held.current.length > 0) recover(snapshot.runId)
    return snapshot
  }, [adopt, apply, cancelId, client, commit, recover, reset, update])

  const cancel = useCallback(() => {
    const id = runId.current
    if (id && runIsActive(live.current.state)) cancelId(id)
  }, [cancelId])

  const { resume } = options
  useEffect(() => {
    if (!resume || runId.current !== null) return
    runId.current = resume
    live.current = { ...IDLE, runId: resume, state: 'running' }
    commit()
    recover(resume)
  }, [commit, recover, resume])

  // Leaving cancels what is still going; a StrictMode remount in between does not.
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      queueMicrotask(() => {
        if (mounted.current) return
        if (frame.current !== null) { cancelAnimationFrame(frame.current); frame.current = null }
        const id = runId.current
        if (id && runIsActive(live.current.state)) cancelId(id)
        reset()
      })
    }
  }, [cancelId, reset])

  return { ...view, active: runIsActive(view.state), start, cancel }
}
