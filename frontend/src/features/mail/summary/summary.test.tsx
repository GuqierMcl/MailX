import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import { testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_ASSISTANT } from '@/features/settings/settings-queries'
import type { AiState, AssistantEvent, AssistantEventKind, EmailDetail, EmailSummarySettings, RunSnapshot } from '@/generated/accounts'
import { i18n } from '@/i18n'
import { MailxClientError } from '@/lib/mailx-client'
import { EmailView } from '../email-view'
import { useEmailSummary } from './use-email-summary'

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, convertFileSrc: () => 'mailx-body://localhost/' }))

const CONNECTION = { id: 'c1', providerId: 'openai', name: 'OpenAI', protocol: 'openaiCompatible' as const, baseUrl: 'https://api.openai.com/v1', useProxy: false, hasKey: true }
const ready: AiState = {
  connections: [CONNECTION], defaultModel: { connectionId: 'c1', modelId: 'big', name: 'Big', contextLength: null, outputLength: null },
  translationModel: null, route: { kind: 'direct' }, proxyPasswordSet: false,
}
const LONG = '会议安排如下。'.repeat(40)

function mail(id = 'mail-1', changes: { text?: string; from?: string; accountId?: string; body?: boolean } = {}): EmailDetail {
  return {
    summary: {
      senderGroup: null, id, accountId: changes.accountId ?? 'a', folderId: 'inbox', subject: `Subject ${id}`,
      from: { name: 'Apple', address: changes.from ?? 'news@apple.com' }, receivedAt: '2026-10-28T00:00:00Z',
      isRead: true, isStarred: false, pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 },
      localOnly: false, archived: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', preview: '',
    },
    to: [], cc: [], replyTo: [], attachments: [], bodyFailure: null,
    body: changes.body === false ? null : { text: changes.text ?? 'hello', hasHtml: false, truncated: false, remoteImagesBlocked: 0 },
  }
}

const snapshot = (over: Partial<RunSnapshot> = {}): RunSnapshot => ({
  runId: 'r1', kind: 'emailSummary', state: 'running', text: '', tools: [], reasoning: [], toolsAvailable: false, error: null, finishedReason: null, completedAt: null, seq: 1, conversationId: null, pending: null, ...over,
})
const event = (seq: number, kind: AssistantEventKind, runId = 'r1'): AssistantEvent => ({ runId, seq, kind })

function Harness({ client, email, onAsk }: { client: ReturnType<typeof testClient>; email: EmailDetail; onAsk: () => void }) {
  const summary = useEmailSummary(client, email)
  return <TooltipProvider>
    <EmailView client={client} email={email} loading={false} error={null} downloading={false} onDownload={vi.fn()} onFlag={vi.fn()}
      onClose={vi.fn()} onRespond={vi.fn()} aiSummary={summary} onAskAi={onAsk} onOpenAiSettings={vi.fn()} />
  </TooltipProvider>
}

async function setup({ summary = {}, excluded = [], kept = null, start = [snapshot()] }: {
  summary?: Partial<EmailSummarySettings>
  excluded?: string[]
  kept?: Awaited<ReturnType<ReturnType<typeof testClient>['assistantEmailSummary']>>
  start?: Array<RunSnapshot | Error>
} = {}) {
  const client = testClient()
  client.aiState.mockResolvedValue(ready)
  await client.updateSettings({ assistant: { ...DEFAULT_ASSISTANT, excludedAccounts: excluded, summary: { ...DEFAULT_ASSISTANT.summary, ...summary } } })
  client.updateSettings.mockClear()
  client.assistantEmailSummary.mockResolvedValue(kept)
  let send: (event: AssistantEvent) => void = () => undefined
  client.onAssistantEvent.mockImplementation((listener) => { send = listener; return () => undefined })
  const queue = [...start]
  client.assistantStart.mockImplementation(async () => {
    const next = queue.length > 1 ? queue.shift()! : queue[0]
    if (next instanceof Error) throw next
    return next
  })
  const emit = (...events: AssistantEvent[]) => act(() => { for (const next of events) send(next) })
  const onAsk = vi.fn()
  const view = render(<Harness client={client} email={mail()} onAsk={onAsk} />)
  const show = (next: EmailDetail) => view.rerender(<Harness client={client} email={next} onAsk={onAsk} />)
  return { client, emit, onAsk, show }
}

const button = () => screen.findByRole('button', { name: 'AI' })
const card = () => screen.queryByRole('region', { name: 'AI 摘要' })

afterEach(() => { vi.restoreAllMocks() })

describe('email summary', () => {
  it('makes a summary from the panel in the chosen style and language and saves the choice', async () => {
    const user = userEvent.setup()
    const { client, emit } = await setup()
    await user.click(await button())
    const panel = await screen.findByRole('dialog', { name: 'AI' })
    // "Summarize" has the focus, so Enter makes it.
    await waitFor(() => expect(within(panel).getByRole('button', { name: '生成摘要' })).toHaveFocus())
    await user.click(within(panel).getByRole('radio', { name: '要点' }))
    await user.click(within(panel).getByRole('button', { name: '生成摘要' }))
    await waitFor(() => expect(client.assistantStart).toHaveBeenCalledWith({
      kind: { type: 'emailSummary', emailId: 'mail-1', style: 'points', language: { mode: 'interface' }, fresh: false },
    }))
    expect(client.updateSettings).toHaveBeenCalledWith({ assistant: expect.objectContaining({ summary: expect.objectContaining({ style: 'points' }) }) })
    await waitFor(() => expect(card()).toBeInTheDocument())
    expect(within(card()!).getByText('正在总结…')).toBeInTheDocument()
    await emit(event(2, { type: 'text', delta: '- 周五前确认。' }), event(3, { type: 'finished', reason: 'stop' }), event(4, { type: 'state', state: 'completed' }))
    await waitFor(() => expect(within(card()!).getByText('周五前确认。')).toBeInTheDocument())
    expect(within(card()!).getByText('· 要点')).toBeInTheDocument()
  })

  it('shows a kept summary on opening without asking the model', async () => {
    const { client } = await setup({ kept: { style: 'brief', language: 'zh-Hans', text: '已经总结过了。', createdAt: '2026-11-19T01:00:00.000Z' } })
    await waitFor(() => expect(card()).toBeInTheDocument())
    expect(within(card()!).getByText('已经总结过了。')).toBeInTheDocument()
    expect(client.assistantEmailSummary).toHaveBeenCalledWith('mail-1')
    expect(client.assistantStart).not.toHaveBeenCalled()
  })

  it('summarizes on its own only when turned on, for long bodies, or for a sender marked always', async () => {
    const off = await setup()
    await button()
    await waitFor(() => expect(off.client.assistantEmailSummary).toHaveBeenCalled())
    expect(off.client.assistantStart).not.toHaveBeenCalled()
    off.show(mail('mail-2', { text: LONG }))
    await waitFor(() => expect(off.client.assistantEmailSummary).toHaveBeenCalledWith('mail-2'))
    expect(off.client.assistantStart).not.toHaveBeenCalled()
  })

  it('auto summary skips short bodies and makes long ones', async () => {
    const { client, show } = await setup({ summary: { auto: true, style: 'detailed' } })
    await waitFor(() => expect(client.assistantEmailSummary).toHaveBeenCalledWith('mail-1'))
    expect(client.assistantStart).not.toHaveBeenCalled()
    show(mail('mail-2', { text: LONG }))
    await waitFor(() => expect(client.assistantStart).toHaveBeenCalledWith({
      kind: { type: 'emailSummary', emailId: 'mail-2', style: 'detailed', language: { mode: 'interface' }, fresh: false },
    }))
  })

  it('a sender marked always is summarized even when short, and the panel toggles the mark', async () => {
    const user = userEvent.setup()
    const { client } = await setup({ summary: { senders: ['news@apple.com'] } })
    await waitFor(() => expect(client.assistantStart).toHaveBeenCalledTimes(1))
    await user.click(await button())
    const box = await screen.findByRole('checkbox', { name: '总是总结来自 Apple 的邮件' })
    expect(box).toBeChecked()
    await user.click(box)
    await waitFor(() => expect(client.updateSettings).toHaveBeenCalledWith({ assistant: expect.objectContaining({ summary: expect.objectContaining({ senders: [] }) }) }))
  })

  it('is not offered without a body or for an account the assistant cannot see', async () => {
    const { show } = await setup({ excluded: ['b'] })
    await button()
    show(mail('mail-2', { body: false }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'AI 摘要' })).not.toBeInTheDocument())
    show(mail('mail-3', { accountId: 'b' }))
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Subject mail-3' })).toBeInTheDocument())
    expect(screen.queryByRole('button', { name: 'AI 摘要' })).not.toBeInTheDocument()
  })

  it('asks for consent in the card and starts again once given', async () => {
    const user = userEvent.setup()
    const { client } = await setup({ start: [snapshot({ state: 'awaitingConsent', seq: 0 }), snapshot({ runId: 'r2' })] })
    client.assistantGrantConsent.mockResolvedValue({ connectionId: 'c1', baseUrl: 'https://api.openai.com/v1' })
    await user.click(await button())
    await user.click(await screen.findByRole('button', { name: '生成摘要' }))
    await waitFor(() => expect(card()).toBeInTheDocument())
    await user.click(within(card()!).getByRole('button', { name: '允许' }))
    await user.click(await screen.findByRole('button', { name: i18n.t('ai:consentConfirm') }))
    await waitFor(() => expect(client.assistantStart).toHaveBeenCalledTimes(2))
  })

  it('a failed run offers a retry, and the card can copy, regenerate, ask, collapse and close', async () => {
    const user = userEvent.setup()
    const { client, emit, onAsk } = await setup({ start: [new MailxClientError({ code: 'aiNetwork', message: '', field: null }), snapshot({ runId: 'r2' }), snapshot({ runId: 'r3' })] })
    await user.click(await button())
    await user.click(await screen.findByRole('button', { name: '生成摘要' }))
    await waitFor(() => expect(within(card()!).getByText(/没能生成摘要/)).toBeInTheDocument())
    await user.click(within(card()!).getByRole('button', { name: '重试' }))
    await emit(event(2, { type: 'text', delta: '一句话。' }, 'r2'), event(3, { type: 'finished', reason: 'stop' }, 'r2'), event(4, { type: 'state', state: 'completed' }, 'r2'))
    await waitFor(() => expect(within(card()!).getByText('一句话。')).toBeInTheDocument())

    await user.click(within(card()!).getByRole('button', { name: '继续问' }))
    expect(onAsk).toHaveBeenCalled()
    await user.click(within(card()!).getByRole('button', { name: '收起' }))
    expect(client.updateSettings).toHaveBeenCalledWith({ assistant: expect.objectContaining({ summary: expect.objectContaining({ collapsed: true }) }) })
    await user.click(within(card()!).getByRole('button', { name: '重新生成' }))
    await waitFor(() => expect(client.assistantStart).toHaveBeenLastCalledWith({
      kind: { type: 'emailSummary', emailId: 'mail-1', style: 'brief', language: { mode: 'interface' }, fresh: true },
    }))
    await user.click(within(card()!).getByRole('button', { name: '关闭摘要' }))
    await waitFor(() => expect(card()).not.toBeInTheDocument())
  })

  it('opening another email stops the summary still being written', async () => {
    const user = userEvent.setup()
    const { client, show } = await setup()
    await user.click(await button())
    await user.click(await screen.findByRole('button', { name: '生成摘要' }))
    await waitFor(() => expect(card()).toBeInTheDocument())
    show(mail('mail-2'))
    await waitFor(() => expect(client.assistantCancel).toHaveBeenCalledWith('r1'))
    await waitFor(() => expect(card()).not.toBeInTheDocument())
  })
})
