import type {
  AssistantEvent,
  ChatTurn,
  ChatTurnConfirmation,
  ChatTurnReasoning,
  ChatTurnTool,
  ConfirmAction,
  ConfirmOutcome,
  ConversationDetail,
  ConversationList,
  ConversationsChanged,
  RunSnapshot,
  StartRun,
} from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'

export type MockConversationsDeps = {
  locale: () => string
  /** `assistant.saveHistory` now. */
  saveHistory: () => boolean
  snapshot: (runId: string) => Promise<RunSnapshot>
  /** Ids the seeded replies can point to; without them the seeds have no cards. */
  firstEmail?: () => { id: string } | null
  firstFolder?: () => { id: string } | null
  /** Replaces the default past conversations. */
  chats?: (zh: boolean) => SeedChat[]
}

export type SeedChat = {
  minutesAgo: number
  turns: { user: string; text: string; tools?: ChatTurnTool[]; reasoning?: ChatTurnReasoning[]; confirmations?: ChatTurnConfirmation[] }[]
}

type Stored = { id: string; title: string; createdAt: string; updatedAt: string; turns: ChatTurn[] }
type Tracked = {
  message: string
  conversationId: string | null
  /** Where in the text each call began (what the interface sees live), and the answers to questions. */
  offsets: Map<string, number>
  asked: Map<string, { callId: string; tool: string; action: ConfirmAction; offset: number }>
  answers: Map<string, ConfirmOutcome>
  length: number
}

const PAGE = 50
const KEEP = 100
const FINISHED = ['completed', 'failed', 'canceled']
const title = (message: string) => {
  const line = message.split(/\s+/).filter(Boolean).join(' ')
  return line.length <= 60 ? line : `${line.slice(0, 60).trimEnd()}…`
}
const missing = () => new MailxClientError({ code: 'notFound', message: '', field: null })

/**
 * Saved chat conversations for the browser preview, in memory for the life of the page. It follows the
 * core's rules: newest first with a cursor, at most 100, a turn is saved when its run ends (unless
 * history is off), a saved turn never carries undo data, and deleting a conversation is final. Two
 * past conversations are there from the start so the history panel has something to show.
 */
export function createMockConversations(deps: MockConversationsDeps) {
  const store: Stored[] = []
  const listeners = new Set<(changed: ConversationsChanged) => void>()
  const tracked = new Map<string, Tracked>()
  const zh = () => deps.locale() === 'zh-CN'
  const changed = (event: ConversationsChanged) => { for (const listener of listeners) listener(structuredClone(event)) }
  const ago = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString()
  let seeded = false
  const newestFirst = () => store.sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : a.updatedAt > b.updatedAt ? -1 : 0))

  function seed() {
    if (seeded) return
    seeded = true
    if (deps.chats) {
      for (const chat of deps.chats(zh())) {
        const turns = chat.turns.flatMap((turn, index): ChatTurn[] => {
          const at = ago(chat.minutesAgo + (chat.turns.length - 1 - index) * 2)
          return [
            { role: 'user', text: turn.user, at },
            {
              role: 'assistant', text: turn.text, tools: turn.tools ?? [], reasoning: turn.reasoning ?? [],
              confirmations: turn.confirmations ?? [], state: 'completed', finishedReason: 'stop', error: null, at,
            },
          ]
        })
        const at = ago(chat.minutesAgo)
        store.push({ id: crypto.randomUUID(), title: title(chat.turns[0].user), createdAt: turns[0].at, updatedAt: at, turns })
      }
      newestFirst()
      return
    }
    const email = deps.firstEmail?.()?.id
    const folder = deps.firstFolder?.()?.id
    const card = email ? `\n\n[${zh() ? '账单提醒' : 'Billing reminder'}](mailx://email/${email})\n\n` : '\n\n'
    const inbox = folder ? `[${zh() ? '收件箱' : 'Inbox'}](mailx://folder/${folder})\n\n` : ''
    const found: ChatTurnTool = { callId: 'call-1', name: 'mail_search', status: 'done', summary: { type: 'found', count: 3 }, textOffset: 0 }
    const archived: ChatTurnTool = { callId: 'call-1', name: 'mail_archive', status: 'done', summary: { type: 'changed', count: 2, emailIds: [], undo: null }, textOffset: 0 }
    const asked: ChatTurnConfirmation = { callId: 'call-1', tool: 'mail_archive', action: 'organize', outcome: 'approved', textOffset: 0 }
    const first = zh() ? '这周有什么要处理的？' : 'What needs my attention this week?'
    const second = zh() ? '把推广邮件归档' : 'Archive the promotional emails'
    store.push(
      {
        id: crypto.randomUUID(), title: title(second), createdAt: ago(60 * 26), updatedAt: ago(60 * 26),
        turns: [
          { role: 'user', text: second, at: ago(60 * 26) },
          { role: 'assistant', text: zh() ? '已经归档了 2 封推广邮件。' : 'Archived 2 promotional emails.', tools: [archived], reasoning: [], confirmations: [asked], state: 'completed', finishedReason: 'stop', error: null, at: ago(60 * 26) },
        ],
      },
      {
        id: crypto.randomUUID(), title: title(first), createdAt: ago(60 * 5), updatedAt: ago(60 * 5),
        turns: [
          { role: 'user', text: first, at: ago(60 * 5) },
          {
            role: 'assistant',
            text: zh() ? `先看账单提醒：${card}${inbox}其余的可以晚点再看。` : `Start with the billing reminder:${card}${inbox}The rest can wait.`,
            tools: [found],
            reasoning: [
              { text: zh() ? '用户想知道这周要处理什么。先搜一下未读和近期的邮件。' : 'The user wants to know what needs attention this week. Search unread and recent mail first.', durationMs: 4200, textOffset: 0, toolsBefore: 0 },
              { text: zh() ? '账单提醒有截止日期，放在最前面。' : 'The billing reminder has a deadline; put it first.', durationMs: 700, textOffset: 0, toolsBefore: 1 },
            ],
            confirmations: [], state: 'completed', finishedReason: 'stop', error: null, at: ago(60 * 5),
          },
        ],
      },
    )
    newestFirst()
  }

  const summary = ({ id, title: name, updatedAt, turns }: Stored) => ({
    id, title: name, updatedAt, messageCount: turns.filter((turn) => turn.role === 'user').length,
  })

  async function save(runId: string) {
    const run = tracked.get(runId)
    tracked.delete(runId)
    if (!run || !deps.saveHistory()) return
    const snapshot = await deps.snapshot(runId).catch(() => null)
    if (!snapshot?.conversationId) return
    const at = snapshot.completedAt ?? new Date().toISOString()
    const tools: ChatTurnTool[] = snapshot.tools.map((tool) => ({
      callId: tool.callId, name: tool.name, status: tool.status, textOffset: run.offsets.get(tool.callId) ?? 0,
      summary: tool.summary.type === 'changed' ? { ...tool.summary, undo: null } : tool.summary,
    }))
    const confirmations: ChatTurnConfirmation[] = [...run.asked.values()].map((question) => ({
      callId: question.callId, tool: question.tool, action: question.action, textOffset: question.offset,
      outcome: run.answers.get(question.callId) ?? 'expired',
    }))
    const reasoning: ChatTurnReasoning[] = snapshot.reasoning.map((thought) => ({
      text: thought.text, durationMs: thought.durationMs ?? 0, textOffset: thought.textOffset, toolsBefore: thought.toolsBefore,
    }))
    const user: ChatTurn = { role: 'user', text: run.message, at }
    const reply: ChatTurn = {
      role: 'assistant', text: snapshot.text, tools, reasoning, confirmations,
      state: snapshot.state === 'completed' ? 'completed' : snapshot.state === 'failed' ? 'failed' : 'canceled',
      finishedReason: snapshot.finishedReason, error: snapshot.error, at,
    }
    seed()
    let conversation = store.find((item) => item.id === snapshot.conversationId)
    if (!conversation) {
      conversation = { id: snapshot.conversationId, title: title(run.message), createdAt: at, updatedAt: at, turns: [] }
      store.push(conversation)
    }
    conversation.turns.push(user, reply)
    conversation.updatedAt = at
    newestFirst()
    store.splice(KEEP)
    changed({ conversationId: conversation.id, change: 'saved' })
  }

  return {
    /** Feed every assistant event here; a chat is saved when its run ends. */
    observe(event: AssistantEvent) {
      const run = tracked.get(event.runId)
      if (!run) return
      const { kind } = event
      if (kind.type === 'text') run.length += kind.delta.length
      else if (kind.type === 'tool') { if (!run.offsets.has(kind.callId)) run.offsets.set(kind.callId, run.length) }
      else if (kind.type === 'confirm') run.asked.set(kind.requestId, { callId: kind.callId, tool: kind.request.tool, action: kind.request.action, offset: run.length })
      else if (kind.type === 'state' && FINISHED.includes(kind.state)) void save(event.runId)
    },
    /** A chat run that started; `request` is what the interface sent. */
    begin(request: StartRun, snapshot: RunSnapshot) {
      if (request.kind.type !== 'chat' || snapshot.state !== 'running') return
      tracked.set(snapshot.runId, { message: request.kind.message.trim(), conversationId: snapshot.conversationId, offsets: new Map(), asked: new Map(), answers: new Map(), length: 0 })
    },
    /** The user answered a question of a run. */
    answered(runId: string, requestId: string, approved: boolean) {
      const run = tracked.get(runId)
      const question = run?.asked.get(requestId)
      if (run && question) run.answers.set(question.callId, approved ? 'approved' : 'denied')
    },
    async list(page?: { limit?: number; cursor?: string | null }): Promise<ConversationList> {
      seed()
      const limit = page?.limit ?? PAGE
      if (limit < 1 || limit > KEEP) throw new MailxClientError({ code: 'validation', message: '', field: 'limit' })
      let start = 0
      if (page?.cursor) {
        const [at, id] = page.cursor.split('|')
        start = store.findIndex((item) => item.id === id && item.updatedAt === at) + 1
        if (start === 0) throw new MailxClientError({ code: 'validation', message: '', field: 'cursor' })
      }
      const items = store.slice(start, start + limit)
      const last = items.at(-1)
      const more = start + limit < store.length
      return { items: items.map(summary), nextCursor: more && last ? `${last.updatedAt}|${last.id}` : null }
    },
    async get(conversationId: string): Promise<ConversationDetail> {
      seed()
      const found = store.find((item) => item.id === conversationId)
      if (!found) throw missing()
      return structuredClone({ id: found.id, title: found.title, createdAt: found.createdAt, updatedAt: found.updatedAt, turns: found.turns })
    },
    async remove(conversationId: string) {
      seed()
      const index = store.findIndex((item) => item.id === conversationId)
      if (index < 0) throw missing()
      store.splice(index, 1)
      changed({ conversationId, change: 'deleted' })
    },
    async clear() {
      seed()
      store.length = 0
      changed({ conversationId: null, change: 'cleared' })
    },
    onChanged(listener: (event: ConversationsChanged) => void) {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
  }
}
