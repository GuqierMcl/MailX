import { useState } from 'react'
import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AiState, AssistantEvent, AssistantEventKind, EmailPage, MailOverview, RunSnapshot } from '@/generated/accounts'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { TooltipProvider } from '@/components/ui/tooltip'
import { MailxClientError } from '@/lib/mailx-client'
import { dayStarts } from './activity'
import { NowPage, type NowPageProps } from './now-page'

const account = exampleAccount({ displayName: '工作邮箱', senderName: '林夕' })
const empty: MailOverview = { newMail: { total: 0, accounts: [], latest: [] }, starredUnread: 0, verificationCodes: [], codeCandidates: [], recentAttachments: [], generatedAt: new Date().toISOString() }
const noMail: EmailPage = { items: [], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }
const CONNECTION = { id: 'c1', providerId: 'openai', name: 'OpenAI', protocol: 'openaiCompatible' as const, baseUrl: 'https://api.openai.com/v1', useProxy: false, hasKey: true }
const ready: AiState = {
  connections: [CONNECTION], defaultModel: { connectionId: 'c1', modelId: 'big', name: 'Big', contextLength: null, outputLength: null },
  translationModel: null, route: { kind: 'direct' }, proxyPasswordSet: false,
}
const SENTENCE = '今天有 3 封新邮件，银行的那封值得先看。'

const snapshot = (over: Partial<RunSnapshot> = {}): RunSnapshot => ({
  runId: 'r1', kind: 'nowSummary', state: 'running', text: '', tools: [], toolsAvailable: true, error: null, finishedReason: null, completedAt: null, seq: 1, conversationId: null, pending: null, ...over,
})
const event = (seq: number, kind: AssistantEventKind, runId = 'r1'): AssistantEvent => ({ runId, seq, kind })

function Harness({ props }: { props: NowPageProps }) {
  const [open, setOpen] = useState(true)
  return <>
    <button type="button" onClick={() => setOpen(false)}>leave</button>
    {open && <NowPage {...props} />}
  </>
}

/** The Now page with an assistant whose events the test sends by hand. */
function setup({ ai = ready, assistant = {}, start = [snapshot()] }: { ai?: AiState; assistant?: Partial<{ enabled: boolean }>; start?: Array<RunSnapshot | Error> } = {}) {
  const client = testClient([account])
  client.getMailOverview.mockResolvedValue(empty)
  client.listEmails.mockResolvedValue(noMail)
  client.aiState.mockResolvedValue(ai)
  if (assistant.enabled === false) {
    const read = client.getSettings.getMockImplementation()!
    client.getSettings.mockImplementation(async () => {
      const current = await read()
      return { ...current, settings: { ...current.settings, assistant: { ...current.settings.assistant, enabled: false } } }
    })
  }
  let send: (event: AssistantEvent) => void = () => undefined
  let settingsChanged: (change: { revision: number }) => void = () => undefined
  client.onSettingsChanged.mockImplementation(async (listener) => { settingsChanged = listener; return () => undefined })
  client.onAssistantEvent.mockImplementation((listener) => { send = listener; return () => undefined })
  const queue = [...start]
  client.assistantStart.mockImplementation(async () => {
    const next = queue.length > 1 ? queue.shift()! : queue[0]
    if (next instanceof Error) throw next
    return next
  })
  const props: NowPageProps = {
    client, accounts: [account], folders: [], syncStates: {}, draftCount: 0, outbox: [],
    verification: { status: () => undefined, busy: () => false, copied: () => false, copy: vi.fn(), requestVisible: () => undefined },
    onSelectMailbox: vi.fn(), onOpenUnread: vi.fn(), onOpenContact: vi.fn(), onOpenAccount: vi.fn(), onOpenEmail: vi.fn(),
  }
  render(<TooltipProvider><Harness props={props} /></TooltipProvider>)
  const emit = (...events: AssistantEvent[]) => act(() => { for (const next of events) send(next) })
  const notifySettings = (revision: number) => act(async () => settingsChanged({ revision }))
  return { client, emit, notifySettings }
}

const preset = () => document.querySelector('[data-slot="now-summary-preset"]')!
const aiLayer = () => document.querySelector('[data-slot="now-summary-ai"]')!
const workingLine = () => document.querySelector('[data-slot="ai-working-line"]')
/** The visible sentence; the same words also sit in the screen-reader status. */
const sentence = (text: string) => within(aiLayer() as HTMLElement).getAllByText(text)[0]
const settled = async () => { await screen.findByRole('heading', { level: 1 }); await waitFor(() => expect(preset()).not.toBeEmptyDOMElement()) }

const reduceMotion = (reduce: boolean) => {
  const original = window.matchMedia
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: reduce && query.includes('reduce'), media: query, onchange: null,
    addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
  }))
  return () => { window.matchMedia = original }
}

afterEach(() => {
  localStorage.clear()
  vi.restoreAllMocks()
})

describe('Now page: AI summary line', () => {
  it('starts one summary run with the page\'s own since and dayStarts once the overview is in', async () => {
    const { client } = setup()
    await settled()
    await waitFor(() => expect(client.assistantStart).toHaveBeenCalledTimes(1))
    const start = new Date()
    start.setHours(0, 0, 0, 0)
    expect(client.assistantStart).toHaveBeenCalledWith({ kind: { type: 'nowSummary', since: start.toISOString(), dayStarts: dayStarts(new Date()), fresh: false } })
    // The overview came first.
    expect(client.getMailOverview.mock.invocationCallOrder[0]).toBeLessThan(client.assistantStart.mock.invocationCallOrder[0])
  })

  it('keeps the preset while it works, then replaces it with the AI sentence, its mark and its time', async () => {
    const { client, emit } = setup()
    await settled()
    const presetText = preset().textContent
    await waitFor(() => expect(workingLine()).toBeInTheDocument())
    expect(workingLine()).toHaveAttribute('data-active', 'true')
    expect(preset()).not.toHaveAttribute('aria-hidden')
    expect(aiLayer()).toHaveAttribute('data-shown', 'false')

    await emit(event(2, { type: 'text', delta: '今天有 3 封新邮件，' }))
    await waitFor(() => expect(aiLayer()).toHaveAttribute('data-shown', 'true'))
    // The preset stays in the layout (same cell) but is hidden from view and from screen readers.
    expect(preset()).toHaveAttribute('aria-hidden', 'true')
    expect(preset().textContent).toBe(presetText)
    expect(preset()).toHaveClass('opacity-0')
    expect(workingLine()).toHaveAttribute('data-active', 'true')
    // Not read out while it streams, and no time until it is done.
    expect(screen.getByRole('status')).toBeEmptyDOMElement()
    expect(screen.queryByRole('img', { name: /生成于/ })).not.toBeInTheDocument()

    await emit(event(3, { type: 'text', delta: '银行的那封值得先看。' }), event(4, { type: 'finished', reason: 'stop' }), event(5, { type: 'state', state: 'completed' }))
    expect(sentence(SENTENCE)).toBeInTheDocument()
    expect(screen.getByRole('status')).toHaveTextContent(SENTENCE)
    const time = new Intl.DateTimeFormat('zh-CN', { hour: '2-digit', minute: '2-digit' }).format(new Date())
    const mark = screen.getByRole('img', { name: `由 AI 生成 · 生成于 ${time}` })
    await userEvent.hover(mark)
    expect((await screen.findAllByText(`由 AI 生成 · 生成于 ${time}`)).length).toBeGreaterThan(0)
    // The working line fades out once done.
    await waitFor(() => expect(workingLine()).not.toBeInTheDocument())
    expect(client.assistantStart).toHaveBeenCalledTimes(1)
  })

  it('shows a cached result at once, with no working treatment', async () => {
    // Generated three hours ago: the line shows that time, not the moment it was fetched.
    const generated = new Date(Date.now() - 3 * 60 * 60 * 1000)
    const { client } = setup({ start: [snapshot({ state: 'completed', text: SENTENCE, seq: 4, finishedReason: 'stop', completedAt: generated.toISOString() })] })
    await settled()
    await waitFor(() => expect(aiLayer()).toHaveAttribute('data-shown', 'true'))
    expect(sentence(SENTENCE)).toBeInTheDocument()
    const time = new Intl.DateTimeFormat('zh-CN', { hour: '2-digit', minute: '2-digit' }).format(generated)
    expect(screen.getByRole('img', { name: `由 AI 生成 · 生成于 ${time}` })).toBeInTheDocument()
    expect(workingLine()).not.toBeInTheDocument()
    expect(client.assistantStart).toHaveBeenCalledTimes(1)
  })

  it('refreshes with a fresh run, keeping the old sentence until the new one is done', async () => {
    const { client, emit } = setup({ start: [snapshot({ state: 'completed', text: SENTENCE, seq: 4 }), snapshot({ runId: 'r2' })] })
    await settled()
    const button = await screen.findByRole('button', { name: '重新生成' })
    // Always there, quiet, with no hover needed.
    expect(button.className).toMatch(/text-muted-foreground\/60/)
    expect(button.className).not.toMatch(/opacity-0/)
    await userEvent.click(button)
    await waitFor(() => expect(client.assistantStart).toHaveBeenCalledTimes(2))
    expect(client.assistantStart).toHaveBeenLastCalledWith({ kind: expect.objectContaining({ type: 'nowSummary', fresh: true }) })
    // The old sentence stays, with the working line under it, and the button waits.
    await waitFor(() => expect(workingLine()).toBeInTheDocument())
    expect(aiLayer()).toHaveAttribute('data-shown', 'true')
    expect(sentence(SENTENCE)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重新生成' })).toBeDisabled()
    expect(document.querySelector('[class*="animate-spin"]')).toBeInTheDocument()
    // Streaming text does not replace it yet.
    await emit(event(2, { type: 'text', delta: '换了' }, 'r2'))
    expect(sentence(SENTENCE)).toBeInTheDocument()
    expect(within(aiLayer() as HTMLElement).queryByText('换了')).not.toBeInTheDocument()
    await emit(event(3, { type: 'text', delta: '一句。' }, 'r2'), event(4, { type: 'state', state: 'completed' }, 'r2'))
    expect(sentence('换了一句。')).toBeInTheDocument()
    expect(within(aiLayer() as HTMLElement).queryByText(SENTENCE)).not.toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: '重新生成' })).toBeEnabled())
    expect(document.querySelector('[class*="animate-spin"]')).not.toBeInTheDocument()
  })

  it('keeps the old sentence when a refresh fails', async () => {
    const { emit } = setup({ start: [snapshot({ state: 'completed', text: SENTENCE, seq: 4 }), snapshot({ runId: 'r2' })] })
    await settled()
    await userEvent.click(await screen.findByRole('button', { name: '重新生成' }))
    await waitFor(() => expect(workingLine()).toBeInTheDocument())
    await emit(event(2, { type: 'text', delta: '半句' }, 'r2'), event(3, { type: 'failed', code: 'aiTimeout' }, 'r2'), event(4, { type: 'state', state: 'failed' }, 'r2'))
    await waitFor(() => expect(workingLine()).not.toBeInTheDocument())
    expect(aiLayer()).toHaveAttribute('data-shown', 'true')
    expect(sentence(SENTENCE)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重新生成' })).toBeEnabled()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('plays one sheen over a finished sentence shown on entry, not over one that streamed in', async () => {
    setup({ start: [snapshot({ state: 'completed', text: SENTENCE, seq: 4 })] })
    await settled()
    await waitFor(() => expect(document.querySelector('[data-slot="now-summary-sheen"]')).toBeInTheDocument())
    expect(document.querySelector('[data-slot="now-summary-sheen"]')).toHaveAttribute('aria-hidden', 'true')
  })

  it('has no sheen after streaming, or with reduced motion', async () => {
    const streamed = setup()
    await settled()
    await waitFor(() => expect(workingLine()).toBeInTheDocument())
    await streamed.emit(event(2, { type: 'text', delta: SENTENCE }), event(3, { type: 'state', state: 'completed' }))
    expect(sentence(SENTENCE)).toBeInTheDocument()
    expect(document.querySelector('[data-slot="now-summary-sheen"]')).not.toBeInTheDocument()
  })

  it('has no sheen with reduced motion', async () => {
    const restore = reduceMotion(true)
    try {
      setup({ start: [snapshot({ state: 'completed', text: SENTENCE, seq: 4 })] })
      await settled()
      await waitFor(() => expect(aiLayer()).toHaveAttribute('data-shown', 'true'))
      expect(document.querySelector('[data-slot="now-summary-sheen"]')).not.toBeInTheDocument()
      expect(document.querySelector('.animate-now-rise')).not.toBeInTheDocument()
    } finally { restore() }
  })

  describe('without consent', () => {
    const awaiting = snapshot({ state: 'awaitingConsent' })

    it('keeps the preset and offers "用 AI 总结"; agreeing starts the run', async () => {
      const { client, emit } = setup({ start: [awaiting, snapshot({ runId: 'r2' })] })
      await settled()
      const offer = await screen.findByRole('button', { name: '用 AI 总结' })
      expect(workingLine()).not.toBeInTheDocument()
      expect(aiLayer()).toHaveAttribute('data-shown', 'false')
      await userEvent.click(offer)
      const dialog = await screen.findByRole('alertdialog')
      expect(within(dialog).getByText('把邮件内容发送给OpenAI？')).toBeInTheDocument()
      expect(within(dialog).getByText(/主题、发件人和正文文字/)).toBeInTheDocument()
      expect(within(dialog).getByText(/附件内容和图片不会发送/)).toBeInTheDocument()
      await waitFor(() => expect(within(dialog).getByRole('button', { name: '取消' })).toHaveFocus())
      expect(client.assistantGrantConsent).not.toHaveBeenCalled()
      await userEvent.click(within(dialog).getByRole('button', { name: '同意' }))
      await waitFor(() => expect(client.assistantGrantConsent).toHaveBeenCalledTimes(1))
      await waitFor(() => expect(client.assistantStart).toHaveBeenCalledTimes(2))
      await waitFor(() => expect(workingLine()).toBeInTheDocument())
      expect(screen.queryByRole('button', { name: '用 AI 总结' })).not.toBeInTheDocument()
      await emit(event(2, { type: 'text', delta: SENTENCE }, 'r2'), event(3, { type: 'state', state: 'completed' }, 'r2'))
      expect(sentence(SENTENCE)).toBeInTheDocument()
    })

    it('does not grant, or start anything, when the dialog is cancelled', async () => {
      const { client } = setup({ start: [awaiting] })
      await settled()
      await userEvent.click(await screen.findByRole('button', { name: '用 AI 总结' }))
      await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '取消' }))
      await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
      expect(client.assistantGrantConsent).not.toHaveBeenCalled()
      expect(client.assistantStart).toHaveBeenCalledTimes(1)
      expect(screen.getByRole('button', { name: '用 AI 总结' })).toBeInTheDocument()
    })

    it('starts by itself once consent arrives from elsewhere (the chat, another window)', async () => {
      const { client, emit, notifySettings } = setup({ start: [awaiting, snapshot({ runId: 'r2' })] })
      await settled()
      await screen.findByRole('button', { name: '用 AI 总结' })
      expect(client.assistantStart).toHaveBeenCalledTimes(1)
      const read = client.getSettings.getMockImplementation()!
      client.getSettings.mockImplementation(async () => {
        const current = await read()
        return { ...current, revision: current.revision + 5, settings: { ...current.settings, assistant: { ...current.settings.assistant, consented: { connectionId: 'c1', baseUrl: 'https://api.openai.com/v1' } } } }
      })
      await notifySettings(1000)
      await waitFor(() => expect(client.assistantStart).toHaveBeenCalledTimes(2))
      await emit(event(2, { type: 'text', delta: SENTENCE }, 'r2'), event(3, { type: 'state', state: 'completed' }, 'r2'))
      expect(sentence(SENTENCE)).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: '用 AI 总结' })).not.toBeInTheDocument()
    })
  })

  describe('failures keep the preset and say nothing', () => {
    it.each(['assistantDisabled', 'aiNotConfigured', 'aiNetwork'] as const)('when the start is refused with %s', async (code) => {
      const { client } = setup({ start: [new MailxClientError({ code, message: 'x', field: null })] })
      await settled()
      await waitFor(() => expect(client.assistantStart).toHaveBeenCalled())
      await act(async () => { await Promise.resolve() })
      expect(aiLayer()).toHaveAttribute('data-shown', 'false')
      expect(preset()).not.toHaveAttribute('aria-hidden')
      expect(workingLine()).not.toBeInTheDocument()
      expect(screen.queryByRole('alert')).not.toBeInTheDocument()
      expect(screen.queryByRole('button', { name: '用 AI 总结' })).not.toBeInTheDocument()
      expect(client.assistantStart).toHaveBeenCalledTimes(1)
    })

    it('when the run fails after some text, the preset comes back', async () => {
      const { emit } = setup()
      await settled()
      await waitFor(() => expect(workingLine()).toBeInTheDocument())
      await emit(event(2, { type: 'text', delta: '今天有' }))
      await waitFor(() => expect(aiLayer()).toHaveAttribute('data-shown', 'true'))
      await emit(event(3, { type: 'failed', code: 'aiTimeout' }), event(4, { type: 'state', state: 'failed' }))
      await waitFor(() => expect(aiLayer()).toHaveAttribute('data-shown', 'false'))
      expect(preset()).not.toHaveAttribute('aria-hidden')
      await waitFor(() => expect(workingLine()).not.toBeInTheDocument())
      expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    })
  })

  it('runs nothing while the assistant is off or no model is usable', async () => {
    const off = setup({ assistant: { enabled: false } })
    await settled()
    await act(async () => { await Promise.resolve() })
    expect(off.client.assistantStart).not.toHaveBeenCalled()
    expect(screen.queryByRole('button', { name: '用 AI 总结' })).not.toBeInTheDocument()
  })

  it('runs nothing without a model', async () => {
    const { client } = setup({ ai: { ...ready, connections: [], defaultModel: null } })
    await settled()
    await act(async () => { await Promise.resolve() })
    expect(client.assistantStart).not.toHaveBeenCalled()
    expect(screen.queryByRole('button', { name: '用 AI 总结' })).not.toBeInTheDocument()
  })

  it('cancels the run when the page is left', async () => {
    const { client } = setup()
    await settled()
    await waitFor(() => expect(workingLine()).toBeInTheDocument())
    await userEvent.click(screen.getByRole('button', { name: 'leave' }))
    await waitFor(() => expect(client.assistantCancel).toHaveBeenCalledWith('r1'))
  })

  describe('reduced motion', () => {
    it('has no animation classes while it works, only a still line and a short fade', async () => {
      const restore = reduceMotion(true)
      try {
        const { emit } = setup()
        await settled()
        await waitFor(() => expect(workingLine()).toBeInTheDocument())
        expect(document.querySelector('.animate-ai-flow')).not.toBeInTheDocument()
        await emit(event(2, { type: 'text', delta: SENTENCE }))
        await waitFor(() => expect(aiLayer()).toHaveAttribute('data-shown', 'true'))
        expect(aiLayer().className).toMatch(/duration-100/)
        expect(aiLayer().className).not.toMatch(/duration-200/)
        expect(document.querySelector('.animate-ai-flow')).not.toBeInTheDocument()
      } finally { restore() }
    })

    it('glides with the flow animation otherwise', async () => {
      setup()
      await settled()
      await waitFor(() => expect(workingLine()).toBeInTheDocument())
      expect(document.querySelector('.animate-ai-flow')).toBeInTheDocument()
    })
  })
})
