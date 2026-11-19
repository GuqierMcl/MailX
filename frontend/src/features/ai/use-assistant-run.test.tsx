import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook, waitFor } from '@/test/render'
import { testClient } from '@/features/accounts/test-fixtures'
import type { AssistantEvent, AssistantEventKind, RunKind, RunSnapshot } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { useAssistantRun } from './use-assistant-run'

const KIND: RunKind = { type: 'nowSummary', since: null, fresh: false }

const snapshot = (over: Partial<RunSnapshot> = {}): RunSnapshot => ({
  runId: 'r1', kind: 'nowSummary', state: 'running', text: '', tools: [], reasoning: [], toolsAvailable: true, error: null, finishedReason: null, completedAt: null, seq: 1, conversationId: null, pending: null, ...over,
})
const event = (seq: number, kind: AssistantEventKind, runId = 'r1'): AssistantEvent => ({ runId, seq, kind })
const text = (seq: number, delta: string, runId = 'r1') => event(seq, { type: 'text', delta }, runId)

/** A client whose events the test sends by hand, the way the host would. */
function setup(first: RunSnapshot = snapshot()) {
  const client = testClient()
  let send: (event: AssistantEvent) => void = () => undefined
  const unsubscribe = vi.fn()
  client.onAssistantEvent.mockImplementation((listener) => { send = listener; return unsubscribe })
  client.assistantStart.mockResolvedValue(first)
  const emit = (...events: AssistantEvent[]) => act(() => { for (const next of events) send(next) })
  const view = renderHook((props: { resume?: string } = {}) => useAssistantRun(client, props))
  return { client, emit, unsubscribe, ...view }
}

afterEach(() => { vi.restoreAllMocks() })

describe('useAssistantRun', () => {
  it('starts a run and follows its events in order', async () => {
    const { client, result, emit } = setup()
    expect(result.current.state).toBe('idle')
    await act(async () => { await result.current.start(KIND) })
    expect(client.assistantStart).toHaveBeenCalledWith({ kind: KIND })
    expect(result.current).toMatchObject({ runId: 'r1', state: 'running', active: true })
    await emit(text(2, 'Hello '), text(3, 'there.'))
    await waitFor(() => expect(result.current.text).toBe('Hello there.'))
    await emit(event(4, { type: 'finished', reason: 'stop' }), event(5, { type: 'state', state: 'completed' }))
    expect(result.current).toMatchObject({ state: 'completed', text: 'Hello there.', finishedReason: 'stop', active: false })
    expect(result.current.finishedAt).toBeInstanceOf(Date)
  })

  it('commits streamed text at most once per animation frame', async () => {
    const { result, emit } = setup()
    await act(async () => { await result.current.start(KIND) })
    const frames: FrameRequestCallback[] = []
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => frames.push(callback))
    await emit(text(2, 'a'), text(3, 'b'), text(4, 'c'))
    // Three deltas, one scheduled frame, nothing shown yet.
    expect(frames).toHaveLength(1)
    expect(result.current.text).toBe('')
    act(() => frames[0](0))
    expect(result.current.text).toBe('abc')
  })

  it('shows a state change together with the text before it', async () => {
    const { result, emit } = setup()
    await act(async () => { await result.current.start(KIND) })
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 1)
    await emit(text(2, 'done'), event(3, { type: 'state', state: 'completed' }))
    expect(result.current).toMatchObject({ state: 'completed', text: 'done' })
  })

  it('opens a thinking stretch at once, streams its words per frame and closes it', async () => {
    const { result, emit } = setup(snapshot({ kind: 'chat' }))
    await act(async () => { await result.current.start(KIND) })
    const frames: FrameRequestCallback[] = []
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => frames.push(callback))
    await emit(event(2, { type: 'reasoning', segment: 0, delta: '' }))
    // The new stretch shows without waiting for a frame.
    expect(result.current.reasoning).toMatchObject([{ segment: 0, text: '', at: 0, toolsBefore: 0, durationMs: null }])
    await emit(event(3, { type: 'reasoning', segment: 0, delta: 'hm' }), event(4, { type: 'reasoning', segment: 0, delta: 'm' }))
    expect(result.current.reasoning[0].text).toBe('')
    expect(frames).toHaveLength(1)
    act(() => frames[0](0))
    expect(result.current.reasoning[0].text).toBe('hmm')
    await emit(event(5, { type: 'reasoningEnd', segment: 0, durationMs: 4200 }), text(6, 'Hi'))
    expect(result.current.reasoning[0].durationMs).toBe(4200)
    // A later stretch after a tool call is placed after it.
    await emit(
      event(7, { type: 'tool', callId: 'c', name: 'mail_search', status: 'running', summary: { type: 'pending' } }),
      event(8, { type: 'reasoning', segment: 1, delta: 'again' }),
      event(9, { type: 'state', state: 'canceled' }),
    )
    expect(result.current.reasoning[1]).toMatchObject({ segment: 1, at: 2, toolsBefore: 1 })
    // Stopped mid-thought: the stretch gets a length so it reads as finished.
    expect(result.current.reasoning[1].durationMs).not.toBeNull()
  })

  it('takes thinking stretches from a snapshot when it recovers', async () => {
    const { client, result, emit } = setup(snapshot({ kind: 'chat' }))
    client.assistantSnapshot.mockResolvedValue(snapshot({
      kind: 'chat', seq: 4, text: 'ab',
      reasoning: [
        { text: 'first', startedAt: '2030-01-01T00:00:00.000Z', durationMs: 1500, textOffset: 0, toolsBefore: 0 },
        { text: 'sec', startedAt: '2030-01-01T00:00:05.000Z', durationMs: null, textOffset: 2, toolsBefore: 1 },
      ],
    }))
    await act(async () => { await result.current.start(KIND) })
    await emit(event(5, { type: 'reasoning', segment: 1, delta: 'ond' }))
    await waitFor(() => expect(result.current.reasoning.map((thought) => thought.text)).toEqual(['first', 'second']))
    expect(result.current.reasoning[0]).toMatchObject({ at: 0, toolsBefore: 0, durationMs: 1500 })
    expect(result.current.reasoning[1]).toMatchObject({ at: 2, toolsBefore: 1, durationMs: null, startedAt: Date.parse('2030-01-01T00:00:05.000Z') })
  })

  it('ignores events at or below the snapshot seq and ones it already has', async () => {
    const { client, result, emit } = setup(snapshot({ seq: 3, text: 'abc' }))
    await act(async () => { await result.current.start(KIND) })
    await emit(text(2, 'old'), text(3, 'old'), text(4, 'd'), text(4, 'd'), text(2, 'late'))
    await waitFor(() => expect(result.current.text).toBe('abcd'))
    expect(client.assistantSnapshot).not.toHaveBeenCalled()
  })

  it('ignores events of other runs', async () => {
    const { result, emit } = setup()
    await act(async () => { await result.current.start(KIND) })
    await emit(text(2, 'x', 'other'), text(2, 'y'))
    await waitFor(() => expect(result.current.text).toBe('y'))
  })

  it('keeps events that arrive before the start answered', async () => {
    const client = testClient()
    let send: (event: AssistantEvent) => void = () => undefined
    client.onAssistantEvent.mockImplementation((listener) => { send = listener; return () => undefined })
    let answer!: (snapshot: RunSnapshot) => void
    client.assistantStart.mockReturnValue(new Promise((resolve) => { answer = resolve }))
    const { result } = renderHook(() => useAssistantRun(client))
    let started!: Promise<unknown>
    act(() => { started = result.current.start(KIND) })
    expect(result.current.state).toBe('starting')
    // seq 2 and 3 are before the answer; the snapshot already holds seq 2.
    act(() => { send(text(2, 'seen')); send(text(3, ' more')) })
    await act(async () => { answer(snapshot({ seq: 2, text: 'seen' })); await started })
    await waitFor(() => expect(result.current.text).toBe('seen more'))
  })

  it('asks for a snapshot when an event is missing, then goes on from it', async () => {
    const { client, result, emit } = setup()
    client.assistantSnapshot.mockResolvedValue(snapshot({ seq: 4, text: 'one two' }))
    await act(async () => { await result.current.start(KIND) })
    // seq 2 never arrives; 4 and 5 do.
    await emit(text(4, 'two'), text(5, '!'))
    await waitFor(() => expect(client.assistantSnapshot).toHaveBeenCalledWith('r1'))
    await waitFor(() => expect(result.current.text).toBe('one two!'))
    expect(client.assistantSnapshot).toHaveBeenCalledTimes(1)
    await emit(text(6, '?'))
    await waitFor(() => expect(result.current.text).toBe('one two!?'))
  })

  it('picks a run up again after a remount from its snapshot, then follows its events', async () => {
    const client = testClient()
    let send: (event: AssistantEvent) => void = () => undefined
    client.onAssistantEvent.mockImplementation((listener) => { send = listener; return () => undefined })
    client.assistantSnapshot.mockResolvedValue(snapshot({ seq: 5, text: 'so far' }))
    const { result } = renderHook(() => useAssistantRun(client, { resume: 'r1' }))
    await waitFor(() => expect(result.current.text).toBe('so far'))
    expect(client.assistantSnapshot).toHaveBeenCalledWith('r1')
    expect(result.current.state).toBe('running')
    act(() => { send(text(5, 'old')); send(text(6, ' and on')) })
    await waitFor(() => expect(result.current.text).toBe('so far and on'))
  })

  it('treats a run the host no longer knows as failed', async () => {
    const client = testClient()
    client.assistantSnapshot.mockRejectedValue(new MailxClientError({ code: 'notFound', message: '', field: null }))
    const { result } = renderHook(() => useAssistantRun(client, { resume: 'gone' }))
    await waitFor(() => expect(result.current.state).toBe('failed'))
  })

  it('reports a refused start as failed with the host\'s code', async () => {
    const { client, result } = setup()
    client.assistantStart.mockRejectedValue(new MailxClientError({ code: 'assistantDisabled', message: '', field: null }))
    await act(async () => { await expect(result.current.start(KIND)).resolves.toBeNull() })
    expect(result.current).toMatchObject({ state: 'failed', error: 'assistantDisabled', active: false })
  })

  it('shows a snapshot that is already complete, as a cached summary arrives', async () => {
    const { client, result } = setup(snapshot({ state: 'completed', text: 'Cached.', seq: 7, finishedReason: 'stop' }))
    await act(async () => { await result.current.start(KIND) })
    expect(result.current).toMatchObject({ state: 'completed', text: 'Cached.', active: false })
    expect(client.assistantCancel).not.toHaveBeenCalled()
  })

  it('stops at awaitingConsent: nothing runs and the run is over', async () => {
    const { result } = setup(snapshot({ state: 'awaitingConsent', seq: 1 }))
    await act(async () => { await result.current.start(KIND) })
    expect(result.current).toMatchObject({ state: 'awaitingConsent', active: false })
  })

  it('cancels the run it follows, keeping the text', async () => {
    const { client, result, emit } = setup()
    await act(async () => { await result.current.start(KIND) })
    await emit(text(2, 'partly'))
    act(() => result.current.cancel())
    expect(client.assistantCancel).toHaveBeenCalledWith('r1')
    await emit(event(3, { type: 'state', state: 'canceled' }))
    expect(result.current).toMatchObject({ state: 'canceled', text: 'partly' })
  })

  it('cancels the run still going when a new one starts', async () => {
    const { client, result } = setup()
    await act(async () => { await result.current.start(KIND) })
    client.assistantStart.mockResolvedValue(snapshot({ runId: 'r2' }))
    await act(async () => { await result.current.start(KIND) })
    expect(client.assistantCancel).toHaveBeenCalledWith('r1')
    expect(result.current.runId).toBe('r2')
  })

  it('cancels a running run on unmount, and stops listening', async () => {
    const { client, result, unmount, unsubscribe } = setup()
    await act(async () => { await result.current.start(KIND) })
    unmount()
    await waitFor(() => expect(client.assistantCancel).toHaveBeenCalledWith('r1'))
    expect(unsubscribe).toHaveBeenCalled()
  })

  it('does not cancel a finished run on unmount', async () => {
    const { client, result, unmount } = setup(snapshot({ state: 'completed', text: 'x', seq: 2 }))
    await act(async () => { await result.current.start(KIND) })
    unmount()
    await act(async () => { await Promise.resolve() })
    expect(client.assistantCancel).not.toHaveBeenCalled()
  })

  it('cancels a run whose start answered after the owner left', async () => {
    const client = testClient()
    let answer!: (snapshot: RunSnapshot) => void
    client.assistantStart.mockReturnValue(new Promise((resolve) => { answer = resolve }))
    const { result, unmount } = renderHook(() => useAssistantRun(client))
    let started!: Promise<unknown>
    act(() => { started = result.current.start(KIND) })
    unmount()
    await act(async () => { await Promise.resolve() })
    await act(async () => { answer(snapshot()); await started })
    expect(client.assistantCancel).toHaveBeenCalledWith('r1')
  })
})
