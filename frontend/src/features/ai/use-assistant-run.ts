import { useCallback, useEffect, useRef, useState } from 'react'
import type { AssistantEvent, ErrorCode, FinishedReason, RunKind, RunSnapshot, RunState } from '@/generated/accounts'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'

/** `idle`: nothing started yet. `starting`: waiting for the host to answer `assistantStart`. */
export type AssistantRunState = 'idle' | 'starting' | RunState

export type AssistantRun = {
  runId: string | null
  state: AssistantRunState
  /** Everything written so far. Committed at most once per animation frame. */
  text: string
  error: ErrorCode | null
  finishedReason: FinishedReason | null
  /** When this window saw the run complete (the host sends no time of its own). */
  finishedAt: Date | null
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

type View = { runId: string | null; state: AssistantRunState; text: string; error: ErrorCode | null; finishedReason: FinishedReason | null; finishedAt: Date | null }

const IDLE: View = { runId: null, state: 'idle', text: '', error: null, finishedReason: null, finishedAt: null }
const ENDED: ReadonlySet<AssistantRunState> = new Set(['completed', 'failed', 'canceled', 'awaitingConsent'])
/** Events seen before `assistantStart` answered are kept briefly, so none is lost to the race. */
const EARLY_LIMIT = 200

export const runIsActive = (state: AssistantRunState) => state === 'starting' || state === 'running' || state === 'awaitingConfirmation'

/** Host-side codes (`hostUnavailable`, `transport`) are not core error codes; they read as internal. */
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
    update({ runId: snapshot.runId, state: snapshot.state, text: snapshot.text, error: snapshot.error, finishedReason: snapshot.finishedReason, finishedAt }, true)
  }, [update])

  const apply = useCallback((event: AssistantEvent) => {
    lastSeq.current = event.seq
    const { kind } = event
    switch (kind.type) {
      case 'text': update({ text: live.current.text + kind.delta }, false); break
      case 'state': update({ state: kind.state, finishedAt: kind.state === 'completed' ? new Date() : live.current.finishedAt }, true); break
      case 'finished': update({ finishedReason: kind.reason }, true); break
      case 'failed': update({ error: kind.code }, true); break
      // Tool activity and confirmations belong to chat views, which come later.
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
