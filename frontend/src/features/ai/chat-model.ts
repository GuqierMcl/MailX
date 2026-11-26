import type { ChatTurn as SavedTurn, ConfirmAction, ConfirmOutcome, ConversationDetail, ErrorCode, FinishedReason } from '@/generated/accounts'
import type { AssistantRunState, RunConfirm, RunThought, RunTool } from './use-assistant-run'

/** What the user answered to a confirmation of a saved reply: only the outcome is kept, never the details. */
export type SavedOutcome = { callId: string; tool: string; action: ConfirmAction; outcome: ConfirmOutcome; at: number }

/** One reply of the assistant, as the chat keeps it for the session. */
export type ChatTurn = {
  state: AssistantRunState
  text: string
  tools: RunTool[]
  /** The model's thinking, stretch by stretch; absent means none. */
  reasoning?: RunThought[]
  confirms: RunConfirm[]
  /** What the user answered to each confirmation, by request id. */
  answers: Record<string, boolean>
  /** The request the run is waiting on, if any. */
  pendingId: string | null
  error: ErrorCode | null
  finishedReason: FinishedReason | null
  finishedAt: Date | null
  /** Set on a reply read back from the saved conversations: its confirmations are one-line outcomes and it cannot be retried. */
  saved?: { outcomes: SavedOutcome[] }
}

export type ChatMessage =
  | { id: string; role: 'user'; text: string; at?: number }
  | { id: string; role: 'assistant'; turn: ChatTurn; at?: number }

/** When a message was said: a reply when it finished, a message when it was sent. */
export function messageTime(message: ChatMessage): Date | null {
  const at = message.role === 'assistant' ? message.turn.finishedAt ?? (message.at ? new Date(message.at) : null) : message.at ? new Date(message.at) : null
  return at && !Number.isNaN(at.getTime()) ? at : null
}

/** Whether a reply changed mail: re-running it would do it a second time, so it is not offered. */
export function turnChangedMail(turn: ChatTurn): boolean {
  const changed = turn.tools.some((tool) => tool.summary.type === 'changed' || tool.summary.type === 'draft' || tool.summary.type === 'outbox')
  return changed || turn.confirms.length > 0 || (turn.saved?.outcomes.length ?? 0) > 0
}

/** Whether a reply stopped short and can be gone on with: stopped, or cut off by a limit. */
export function turnCanContinue(turn: ChatTurn): boolean {
  return turn.state === 'canceled' || turn.finishedReason === 'limit' || turn.finishedReason === 'length'
}

/**
 * A reply that was gone on with, shown as one: the new run's parts follow the old ones, its text
 * after the old text (offsets and thinking stretches moved along), its state the new run's.
 */
export function joinTurns(before: ChatTurn, after: ChatTurn): ChatTurn {
  const shift = before.text.length
  const segments = Math.max(-1, ...(before.reasoning ?? []).map((thought) => thought.segment)) + 1
  const outcomes = before.saved?.outcomes ?? []
  return {
    ...after,
    text: before.text + after.text,
    tools: [...before.tools, ...after.tools.map((tool) => ({ ...tool, at: tool.at + shift }))],
    reasoning: [
      ...(before.reasoning ?? []),
      ...(after.reasoning ?? []).map((thought) => ({
        ...thought, segment: thought.segment + segments, at: thought.at + shift, toolsBefore: thought.toolsBefore + before.tools.length,
      })),
    ],
    confirms: [...before.confirms, ...after.confirms.map((confirm) => ({ ...confirm, at: confirm.at + shift }))],
    answers: { ...before.answers, ...after.answers },
    ...(outcomes.length > 0 ? { saved: { outcomes } } : { saved: undefined }),
  }
}

/** The parts of a reply in the order they came. */
export type Fragment =
  | { key: string; kind: 'text'; text: string; streaming: boolean }
  | { key: string; kind: 'thought'; thought: RunThought }
  | { key: string; kind: 'tool'; tool: RunTool }
  | { key: string; kind: 'confirm'; confirm: RunConfirm }
  | { key: string; kind: 'outcome'; outcome: SavedOutcome }

export type ConfirmStatus = 'waiting' | 'approved' | 'denied' | 'closed'

export const turnWorking = (turn: ChatTurn) => turn.state === 'starting' || turn.state === 'running' || turn.state === 'awaitingConfirmation'

/** Whether the user answered, the run still waits, or it ended without an answer (cancelled, timed out). */
export function confirmStatus(turn: ChatTurn, requestId: string): ConfirmStatus {
  const answer = turn.answers[requestId]
  if (answer !== undefined) return answer ? 'approved' : 'denied'
  return turn.pendingId === requestId && turn.state === 'awaitingConfirmation' ? 'waiting' : 'closed'
}

type Part = { at: number; order: number } & (
  | { kind: 'thought'; thought: RunThought }
  | { kind: 'tool'; tool: RunTool }
  | { kind: 'confirm'; confirm: RunConfirm }
  | { kind: 'outcome'; outcome: SavedOutcome })

/**
 * Text, thinking, tool calls and confirmations in the order they began. A tool whose confirmation card is
 * shown has no row of its own: the card says what happened to it. Only the text after the last part grows.
 * Parts at the same text offset keep the order they came in: a thinking stretch goes before the tool
 * calls that began after it (`toolsBefore`), a confirmation takes its call's place.
 */
export function turnFragments(turn: ChatTurn): Fragment[] {
  const outcomes = turn.saved?.outcomes ?? []
  const confirmed = new Set([...turn.confirms.map((confirm) => confirm.callId), ...outcomes.map((outcome) => outcome.callId)])
  const index = new Map(turn.tools.map((tool, at) => [tool.callId, at]))
  const callOrder = (callId: string) => 2 * (index.get(callId) ?? turn.tools.length) + 1
  const parts: Part[] = [
    ...(turn.reasoning ?? []).map((thought): Part => ({ at: thought.at, order: 2 * thought.toolsBefore, kind: 'thought', thought })),
    ...turn.tools.filter((tool) => !confirmed.has(tool.callId)).map((tool): Part => ({ at: tool.at, order: callOrder(tool.callId), kind: 'tool', tool })),
    ...turn.confirms.map((confirm): Part => ({ at: confirm.at, order: callOrder(confirm.callId), kind: 'confirm', confirm })),
    ...outcomes.map((outcome): Part => ({ at: outcome.at, order: callOrder(outcome.callId), kind: 'outcome', outcome })),
  ].sort((a, b) => a.at - b.at || a.order - b.order)
  const out: Fragment[] = []
  const live = turnWorking(turn)
  let from = 0
  const text = (to: number, last: boolean) => {
    const slice = turn.text.slice(from, to)
    if (slice.trim() !== '') out.push({ key: `text-${from}`, kind: 'text', text: slice, streaming: last && live })
    from = Math.max(from, to)
  }
  for (const part of parts) {
    text(Math.min(part.at, turn.text.length), false)
    if (part.kind === 'thought') out.push({ key: `thought-${part.thought.segment}`, kind: 'thought', thought: part.thought })
    else if (part.kind === 'tool') out.push({ key: `tool-${part.tool.callId}`, kind: 'tool', tool: part.tool })
    else if (part.kind === 'confirm') out.push({ key: `confirm-${part.confirm.requestId}`, kind: 'confirm', confirm: part.confirm })
    else out.push({ key: `outcome-${part.outcome.callId}`, kind: 'outcome', outcome: part.outcome })
  }
  text(turn.text.length, true)
  return out
}

/** Emails a tool call touched or read, once each, in order. */
export function toolEmailIds(tool: RunTool): string[] {
  const { summary } = tool
  if (summary.type === 'read' || summary.type === 'changed') return [...new Set(summary.emailIds)]
  return []
}

/** A saved reply as the chat shows it. `textOffset` puts tools, thinking and answered confirmations back where they were live. */
function savedReply(turn: Extract<SavedTurn, { role: 'assistant' }>): ChatTurn {
  const finished = new Date(turn.at)
  return {
    state: turn.state, text: turn.text,
    // Saved activities carry no undo (the host drops it); `settledAt` null keeps any undo window closed.
    tools: turn.tools.map((tool) => ({ callId: tool.callId, name: tool.name, status: tool.status, summary: tool.summary, at: tool.textOffset, settledAt: null })),
    reasoning: turn.reasoning.map((thought, segment) => ({
      segment, text: thought.text, at: thought.textOffset, toolsBefore: thought.toolsBefore, startedAt: 0, durationMs: thought.durationMs,
    })),
    confirms: [], answers: {}, pendingId: null, error: turn.error, finishedReason: turn.finishedReason,
    finishedAt: turn.state === 'completed' && !Number.isNaN(finished.getTime()) ? finished : null,
    saved: { outcomes: turn.confirmations.map((item) => ({ callId: item.callId, tool: item.tool, action: item.action, outcome: item.outcome, at: item.textOffset })) },
  }
}

/** The messages of a saved conversation, oldest first, in the model the live chat renders. */
export function savedMessages(detail: ConversationDetail): ChatMessage[] {
  return detail.turns.map((turn, index): ChatMessage => {
    const at = Date.parse(turn.at)
    return turn.role === 'user'
      ? { id: `saved-${index}`, role: 'user', text: turn.text, ...(Number.isNaN(at) ? {} : { at }) }
      : { id: `saved-${index}`, role: 'assistant', turn: savedReply(turn), ...(Number.isNaN(at) ? {} : { at }) }
  })
}
