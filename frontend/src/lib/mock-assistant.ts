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
        if (over(kind.state)) snap.completedAt = new Date().toISOString()
        if (kind.state !== 'awaitingConfirmation') snap.pending = null
        break
      case 'text': snap.text += kind.delta; break
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

  async function chat(run: Run, message: string) {
    if (/归档|archive/i.test(message)) {
      const target = deps.firstUnread()
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
      // A search, a read, then a reply with a list and a link.
      const target = deps.firstUnread()
      const { unread } = deps.overview()
      emit(run, { type: 'tool', callId: 'call-1', name: 'search_emails', status: 'running', summary: { type: 'pending' } })
      await sleep()
      emit(run, { type: 'tool', callId: 'call-1', name: 'search_emails', status: 'done', summary: { type: 'found', count: Math.max(unread, 3) } })
      if (target) {
        emit(run, { type: 'tool', callId: 'call-2', name: 'read_email', status: 'running', summary: { type: 'pending' } })
        await sleep()
        emit(run, { type: 'tool', callId: 'call-2', name: 'read_email', status: 'done', summary: { type: 'read', emailIds: [target.id] } })
      }
      await say(run, zh()
        ? `收件箱里有 ${unread} 封未读邮件。先看这几项：\n\n- **账单提醒**：这周内要处理\n- 一封需要回复的 *项目* 邮件\n- 几封可以直接归档的通知\n\n想了解更多，可以看[帮助文档](https://example.com/help)，或者说“归档”“写草稿”“发送”试试。`
        : `You have ${unread} unread emails. Start with these:\n\n- **Billing reminder**: due this week\n- A *project* email that needs a reply\n- A few notifications you can archive\n\nSee the [help page](https://example.com/help) for more, or say "archive", "draft" or "send" to try them.`)
    }
    finish(run)
  }

  function create(kind: RunSnapshot['kind'], state: RunSnapshot['state'], conversationId: string | null): Run {
    const run: Run = {
      waiting: null,
      snap: { runId: crypto.randomUUID(), kind, state: 'running', text: '', tools: [], toolsAvailable: kind === 'chat', error: null, finishedReason: null, completedAt: null, seq: 0, conversationId, pending: null },
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
