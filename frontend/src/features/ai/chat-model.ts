import type { ChatTurn as SavedTurn, ConfirmAction, ConfirmOutcome, ConversationDetail, ErrorCode, FinishedReason } from '@/generated/accounts'
import type { AssistantRunState, RunConfirm, RunTool } from './use-assistant-run'

/** What the user answered to a confirmation of a saved reply: only the outcome is kept, never the details. */
export type SavedOutcome = { callId: string; tool: string; action: ConfirmAction; outcome: ConfirmOutcome; at: number }

/** One reply of the assistant, as the chat keeps it for the session. */
export type ChatTurn = {
  state: AssistantRunState
  text: string
  tools: RunTool[]
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
  | { id: string; role: 'user'; text: string }
  | { id: string; role: 'assistant'; turn: ChatTurn }

/** The parts of a reply in the order they came. */
export type Fragment =
  | { key: string; kind: 'text'; text: string; streaming: boolean }
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

/**
 * Text, tool calls and confirmations in the order they began. A tool whose confirmation card is shown
 * has no row of its own: the card says what happened to it. Only the text after the last part grows.
 */
export function turnFragments(turn: ChatTurn): Fragment[] {
  const outcomes = turn.saved?.outcomes ?? []
  const confirmed = new Set([...turn.confirms.map((confirm) => confirm.callId), ...outcomes.map((outcome) => outcome.callId)])
  const parts: ({ at: number } & ({ kind: 'tool'; tool: RunTool } | { kind: 'confirm'; confirm: RunConfirm } | { kind: 'outcome'; outcome: SavedOutcome }))[] = [
    ...turn.tools.filter((tool) => !confirmed.has(tool.callId)).map((tool) => ({ at: tool.at, kind: 'tool' as const, tool })),
    ...turn.confirms.map((confirm) => ({ at: confirm.at, kind: 'confirm' as const, confirm })),
    ...outcomes.map((outcome) => ({ at: outcome.at, kind: 'outcome' as const, outcome })),
  ].sort((a, b) => a.at - b.at)
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
    if (part.kind === 'tool') out.push({ key: `tool-${part.tool.callId}`, kind: 'tool', tool: part.tool })
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

/** A saved reply as the chat shows it. `textOffset` puts tools and answered confirmations back where they were live. */
function savedReply(turn: Extract<SavedTurn, { role: 'assistant' }>): ChatTurn {
  const finished = new Date(turn.at)
  return {
    state: turn.state, text: turn.text,
    // Saved activities carry no undo (the host drops it); `settledAt` null keeps any undo window closed.
    tools: turn.tools.map((tool) => ({ callId: tool.callId, name: tool.name, status: tool.status, summary: tool.summary, at: tool.textOffset, settledAt: null })),
    confirms: [], answers: {}, pendingId: null, error: turn.error, finishedReason: turn.finishedReason,
    finishedAt: turn.state === 'completed' && !Number.isNaN(finished.getTime()) ? finished : null,
    saved: { outcomes: turn.confirmations.map((item) => ({ callId: item.callId, tool: item.tool, action: item.action, outcome: item.outcome, at: item.textOffset })) },
  }
}

/** The messages of a saved conversation, oldest first, in the model the live chat renders. */
export function savedMessages(detail: ConversationDetail): ChatMessage[] {
  return detail.turns.map((turn, index): ChatMessage => turn.role === 'user'
    ? { id: `saved-${index}`, role: 'user', text: turn.text }
    : { id: `saved-${index}`, role: 'assistant', turn: savedReply(turn) })
}
