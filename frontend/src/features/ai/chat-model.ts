import type { ErrorCode, FinishedReason } from '@/generated/accounts'
import type { AssistantRunState, RunConfirm, RunTool } from './use-assistant-run'

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
}

export type ChatMessage =
  | { id: string; role: 'user'; text: string }
  | { id: string; role: 'assistant'; turn: ChatTurn }

/** The parts of a reply in the order they came. */
export type Fragment =
  | { key: string; kind: 'text'; text: string; streaming: boolean }
  | { key: string; kind: 'tool'; tool: RunTool }
  | { key: string; kind: 'confirm'; confirm: RunConfirm }

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
  const confirmed = new Set(turn.confirms.map((confirm) => confirm.callId))
  const parts: ({ at: number } & ({ kind: 'tool'; tool: RunTool } | { kind: 'confirm'; confirm: RunConfirm }))[] = [
    ...turn.tools.filter((tool) => !confirmed.has(tool.callId)).map((tool) => ({ at: tool.at, kind: 'tool' as const, tool })),
    ...turn.confirms.map((confirm) => ({ at: confirm.at, kind: 'confirm' as const, confirm })),
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
    out.push(part.kind === 'tool' ? { key: `tool-${part.tool.callId}`, kind: 'tool', tool: part.tool } : { key: `confirm-${part.confirm.requestId}`, kind: 'confirm', confirm: part.confirm })
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
