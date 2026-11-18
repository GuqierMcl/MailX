import type {
  AiConsent,
  AssistantEvent,
  AssistantEventKind,
  Draft,
  OutboxItem,
  RunSnapshot,
  StartRun,
} from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'

/** The consent the browser preview records; the real one names the connection the user agreed to. */
export const MOCK_CONSENT: AiConsent = { connectionId: '00000000-0000-4000-8000-0000000000a1', baseUrl: 'https://api.mock.example/v1' }

export type MockAssistantDeps = {
  /** Delay between streamed events. */
  stepMs: number
  locale: () => string
  overview: () => { unread: number; starred: number }
  firstUnread: () => { id: string } | null
  /** A folder the reply can point to (its id as the folder tree has it); without it the reply has no folder card. */
  firstFolder?: () => { id: string } | null
  /** Archives the emails and returns the undo token the host would hand to the UI. */
  archive: (emailIds: string[]) => Promise<string | null>
  /** A draft addressed to someone new, as the assistant would have written it. */
  draftToSend: () => Promise<{ draft: Draft; accountName: string; accountAddress: string }>
  send: (draft: Draft) => Promise<OutboxItem>
  /** The connection the assistant would use now, as the core's `assistant_binding`; `null` without a model. */
  binding?: () => AiConsent | null
}

type Run = { snap: RunSnapshot; waiting: ((approved: boolean) => void) | null }

const over = (state: RunSnapshot['state']) => state === 'completed' || state === 'failed' || state === 'canceled' || state === 'awaitingConsent'
const failure = (code: 'notFound' | 'conflict' | 'validation', field: string | null = null) =>
  new MailxClientError({ code, message: '', field })

/**
 * A small stand-in for the core's assistant, for designing the UI in a browser. It follows the same
 * rules as the real one: events in `seq` order, a consent before anything runs, one pending confirmation
 * per run that only that request id can answer, once. The "model" is a few canned replies.
 */
export function createMockAssistant(deps: MockAssistantDeps) {
  const runs = new Map<string, Run>()
  const listeners = new Set<(event: AssistantEvent) => void>()
  let consent: AiConsent | null = null
  const zh = () => deps.locale() === 'zh-CN'
  const sleep = () => new Promise<void>((resolve) => setTimeout(resolve, deps.stepMs))

  function emit(run: Run, kind: AssistantEventKind): boolean {
    const snap = run.snap
    if (over(snap.state)) return false
    snap.seq += 1
    switch (kind.type) {
      case 'state':
        snap.state = kind.state
        if (over(kind.state)) {
          snap.completedAt = new Date().toISOString()
          for (const thought of snap.reasoning) thought.durationMs ??= Math.max(0, Date.now() - Date.parse(thought.startedAt))
        }
        if (kind.state !== 'awaitingConfirmation') snap.pending = null
        break
      case 'text': snap.text += kind.delta; break
      case 'reasoning': {
        const open = snap.reasoning[kind.segment]
        if (open) open.text += kind.delta
        else snap.reasoning.push({ text: kind.delta, startedAt: new Date().toISOString(), durationMs: null, textOffset: snap.text.length, toolsBefore: snap.tools.length })
        break
      }
      case 'reasoningEnd': {
        const open = snap.reasoning[kind.segment]
        if (open) open.durationMs = kind.durationMs
        break
      }
      case 'tool': {
        const activity = { callId: kind.callId, name: kind.name, status: kind.status, summary: kind.summary }
        const index = snap.tools.findIndex((tool) => tool.callId === kind.callId)
        if (index >= 0) snap.tools[index] = activity
        else snap.tools.push(activity)
        break
      }
      case 'confirm': snap.pending = { requestId: kind.requestId, callId: kind.callId, request: kind.request }; break
      case 'finished': snap.finishedReason = kind.reason; break
      case 'failed': snap.error = kind.code; break
    }
    const event: AssistantEvent = { runId: snap.runId, seq: snap.seq, kind }
    for (const listener of listeners) listener(structuredClone(event))
    return true
  }

  const finish = (run: Run) => {
    if (emit(run, { type: 'finished', reason: 'stop' })) emit(run, { type: 'state', state: 'completed' })
  }
  const alive = (run: Run) => !over(run.snap.state)

  async function say(run: Run, text: string) {
    const letters = Array.from(text)
    for (let at = 0; at < letters.length && alive(run); at += 6) {
      await sleep()
      emit(run, { type: 'text', delta: letters.slice(at, at + 6).join('') })
    }
  }

  /** A stretch of thinking before the next words or step: streamed a few words at a time, then closed. */
  async function think(run: Run, text: string) {
    const segment = run.snap.reasoning.length
    const started = Date.now()
    const words = text.split(/(?<=[\s，。,.])/)
    emit(run, { type: 'reasoning', segment, delta: '' })
    for (let at = 0; at < words.length && alive(run); at += 2) {
      await sleep()
      emit(run, { type: 'reasoning', segment, delta: words.slice(at, at + 2).join('') })
    }
    // Long enough to read "思考了 N 秒" in the preview.
    await sleep()
    emit(run, { type: 'reasoningEnd', segment, durationMs: Date.now() - started })
  }

  async function ask(run: Run, callId: string, request: Extract<AssistantEventKind, { type: 'confirm' }>['request']) {
    const answer = new Promise<boolean>((resolve) => { run.waiting = resolve })
    emit(run, { type: 'state', state: 'awaitingConfirmation' })
    emit(run, { type: 'confirm', requestId: crypto.randomUUID(), callId, request })
    return answer
  }

  async function nowSummary(run: Run) {
    const { unread, starred } = deps.overview()
    await say(run, zh()
      ? `收件箱里有 ${unread} 封未读邮件，其中 ${starred} 封加了星标。`
      : `You have ${unread} unread emails, ${starred} of them starred.`)
    finish(run)
  }

  /** A reply that uses most of the Markdown the chat can show, to preview it while it streams. */
  function markdownShowcase(): string {
    const email = deps.firstUnread()?.id
    const card = email ? `[${zh() ? '账单提醒' : 'Billing reminder'}](mailx://email/${email})\n\n` : ''
    return zh()
      ? `## 本周待办\n\n按紧急程度排好了，~~周一的会议~~已经取消：\n\n1. **付账单**\n\n   周五前处理，金额见下表。\n2. 回复项目邮件\n   - 确认排期\n   - 附上 \`roadmap.pdf\`\n\n| 事项 | 截止 | 金额 |\n|:--|:-:|--:|\n| 账单 | 周五 | ¥1,280.00 |\n| 续费 | 下周一 | ¥96.00 |\n\n- [x] 读完周报\n- [ ] 回复王五\n\n> 引用的原文放在这里。\n\n### 过滤规则示例\n\n\`\`\`sieve\nif header :contains "subject" "账单" {\n  fileinto "Bills";\n}\n\`\`\`\n\n---\n\n${card}详细说明见 https://example.com/help。`
      : `## This week\n\nSorted by urgency; ~~Monday's meeting~~ was canceled:\n\n1. **Pay the bill**\n\n   Due Friday, amounts below.\n2. Reply to the project email\n   - confirm the dates\n   - attach \`roadmap.pdf\`\n\n| Item | Due | Amount |\n|:--|:-:|--:|\n| Bill | Fri | $1,280.00 |\n| Renewal | Mon | $96.00 |\n\n- [x] Read the weekly report\n- [ ] Reply to Wang\n\n> The quoted original goes here.\n\n### A filter rule\n\n\`\`\`sieve\nif header :contains "subject" "bill" {\n  fileinto "Bills";\n}\n\`\`\`\n\n---\n\n${card}See https://example.com/help for details.`
  }

  async function chat(run: Run, message: string) {
    if (/markdown|格式|排版|表格/i.test(message)) {
      await say(run, markdownShowcase())
    } else if (/归档|archive/i.test(message)) {
      const target = deps.firstUnread()
      await think(run, zh() ? '用户要归档邮件。先找第一封未读的。' : 'The user wants mail archived. Find the first unread one.')
      await say(run, zh() ? '好的，我来归档。' : 'Sure, archiving.')
      if (!target) { await say(run, zh() ? '没有可归档的邮件。' : 'There is nothing to archive.'); return finish(run) }
      emit(run, { type: 'tool', callId: 'call-1', name: 'archive_emails', status: 'running', summary: { type: 'pending' } })
      await sleep()
      const token = await deps.archive([target.id])
      emit(run, {
        type: 'tool', callId: 'call-1', name: 'archive_emails', status: 'done',
        summary: { type: 'changed', count: 1, emailIds: [target.id], undo: token ? { type: 'archive', token } : null },
      })
      await say(run, zh() ? '归档了 1 封。' : 'Archived 1 email.')
    } else if (/发|send/i.test(message)) {
      const { draft, accountName, accountAddress } = await deps.draftToSend()
      await say(run, zh() ? '草稿写好了，现在发送。' : 'The draft is ready, sending it now.')
      emit(run, { type: 'tool', callId: 'call-1', name: 'send_email', status: 'running', summary: { type: 'pending' } })
      const approved = await ask(run, 'call-1', {
        action: 'send',
        tool: 'send_email',
        details: {
          type: 'send', accountName, accountAddress, to: draft.to, cc: draft.cc, bcc: draft.bcc, subject: draft.subject,
          bodyPreview: draft.bodyText.slice(0, 300), attachments: [], warnings: ['newRecipient', 'notInConversation'],
        },
      })
      if (!alive(run)) return
      if (approved) {
        const item = await deps.send(draft)
        emit(run, { type: 'tool', callId: 'call-1', name: 'send_email', status: 'done', summary: { type: 'outbox', outboxId: item.id } })
        await say(run, zh() ? '已加入发送队列。' : 'It is in the send queue.')
      } else {
        emit(run, { type: 'tool', callId: 'call-1', name: 'send_email', status: 'denied', summary: { type: 'denied' } })
        await say(run, zh() ? '好的，没有发送。' : 'Okay, I did not send it.')
      }
    } else if (/草稿|draft/i.test(message) && !/发|send/i.test(message)) {
      const { draft } = await deps.draftToSend()
      emit(run, { type: 'tool', callId: 'call-1', name: 'create_draft', status: 'running', summary: { type: 'pending' } })
      await sleep()
      emit(run, { type: 'tool', callId: 'call-1', name: 'create_draft', status: 'done', summary: { type: 'draft', draftId: draft.id, subject: draft.subject } })
      await say(run, zh() ? '草稿放在草稿箱里了，你可以打开看看，改好再发。' : 'The draft is in your drafts. Open it, edit if you like, then send.')
    } else {
      // Thinks, searches and reads, thinks again, then a reply with a list and a link.
      const target = deps.firstUnread()
      const { unread } = deps.overview()
      await think(run, zh()
        ? '用户想知道邮箱里有什么要处理。先搜索未读邮件，再读最急的那封，看看有没有截止日期。\n\n搜索结果出来后按紧急程度排序。'
        : 'The user wants to know what needs attention. Search unread mail first, then read the most urgent one and look for a deadline.\n\nOnce the results are in, sort them by urgency.')
      emit(run, { type: 'tool', callId: 'call-1', name: 'search_emails', status: 'running', summary: { type: 'pending' } })
      await sleep()
      emit(run, { type: 'tool', callId: 'call-1', name: 'search_emails', status: 'done', summary: { type: 'found', count: Math.max(unread, 3) } })
      if (target) {
        emit(run, { type: 'tool', callId: 'call-2', name: 'read_email', status: 'running', summary: { type: 'pending' } })
        await sleep()
        emit(run, { type: 'tool', callId: 'call-2', name: 'read_email', status: 'done', summary: { type: 'read', emailIds: [target.id] } })
      }
      await think(run, zh() ? '账单提醒这周到期，放在第一条；其余可以晚点看。' : 'The billing reminder is due this week, so it goes first; the rest can wait.')
      const email = target?.id
      const folder = deps.firstFolder?.()?.id
      // Links to local mail and folders: a card on a line of its own, a chip inside a sentence.
      const emailCard = email ? `[${zh() ? '账单提醒' : 'Billing reminder'}](mailx://email/${email})\n\n` : ''
      const emailChip = email ? (zh() ? `比较急的是[这封邮件](mailx://email/${email})。\n\n` : `The urgent one is [this email](mailx://email/${email}).\n\n`) : ''
      const folderCard = folder ? `[${zh() ? '收件箱' : 'Inbox'}](mailx://folder/${folder})\n\n` : ''
      await say(run, zh()
        ? `收件箱里有 ${unread} 封未读邮件。先看这几项：\n\n- **账单提醒**：这周内要处理\n- 一封需要回复的 *项目* 邮件\n- 几封可以直接归档的通知\n\n${emailCard}${emailChip}${folderCard}想了解更多，可以看[帮助文档](https://example.com/help)，或者说“归档”“写草稿”“发送”试试。`
        : `You have ${unread} unread emails. Start with these:\n\n- **Billing reminder**: due this week\n- A *project* email that needs a reply\n- A few notifications you can archive\n\n${emailCard}${emailChip}${folderCard}See the [help page](https://example.com/help) for more, or say "archive", "draft" or "send" to try them.`)
    }
    finish(run)
  }

  function create(kind: RunSnapshot['kind'], state: RunSnapshot['state'], conversationId: string | null): Run {
    const run: Run = {
      waiting: null,
      snap: { runId: crypto.randomUUID(), kind, state: 'running', text: '', tools: [], reasoning: [], toolsAvailable: kind === 'chat', error: null, finishedReason: null, completedAt: null, seq: 0, conversationId, pending: null },
    }
    runs.set(run.snap.runId, run)
    emit(run, { type: 'state', state })
    return run
  }

  const existing = (runId: string) => {
    const run = runs.get(runId)
    if (!run) throw failure('notFound')
    return run
  }

  return {
    consent: () => consent,
    onEvent(listener: (event: AssistantEvent) => void) {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    async start(request: StartRun): Promise<RunSnapshot> {
      const { kind } = request
      if (kind.type === 'chat' && (!kind.message.trim() || kind.message.length > 20_000)) throw failure('validation', 'message')
      const tag = kind.type
      const conversationId = kind.type === 'chat' ? kind.conversationId ?? crypto.randomUUID() : null
      // Like the core: the consent must name the connection in use now; switching connections asks again.
      const binding = deps.binding?.() ?? MOCK_CONSENT
      if (!consent || consent.connectionId !== binding.connectionId || consent.baseUrl !== binding.baseUrl) return structuredClone(create(tag, 'awaitingConsent', conversationId).snap)
      const run = create(tag, 'running', conversationId)
      const body = kind.type === 'chat' ? chat(run, kind.message.trim()) : nowSummary(run)
      void body.catch(() => { emit(run, { type: 'failed', code: 'internal' }); emit(run, { type: 'state', state: 'failed' }) })
      return structuredClone(run.snap)
    },
    async cancel(runId: string) {
      const run = existing(runId)
      if (emit(run, { type: 'state', state: 'canceled' })) run.waiting?.(false)
      run.waiting = null
    },
    async snapshot(runId: string) { return structuredClone(existing(runId).snap) },
    async respond(runId: string, requestId: string, approved: boolean) {
      const run = existing(runId)
      if (!run.waiting || run.snap.state !== 'awaitingConfirmation' || run.snap.pending?.requestId !== requestId) throw failure('conflict')
      const answer = run.waiting
      run.waiting = null
      emit(run, { type: 'state', state: 'running' })
      answer(approved)
    },
    async grantConsent() { consent = deps.binding?.() ?? MOCK_CONSENT; return structuredClone(consent) },
    async revokeConsent() {
      consent = null
      for (const run of runs.values()) {
        if (emit(run, { type: 'state', state: 'canceled' })) run.waiting?.(false)
      }
    },
  }
}
