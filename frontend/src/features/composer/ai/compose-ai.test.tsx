import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import type { MailBodyEditorHandle } from '../mail-body-editor'
import { testClient } from '@/features/accounts/test-fixtures'
import type { ComposeTask, Draft, DraftKind, RunSnapshot } from '@/generated/accounts'
import { mockDraftBody } from '@/lib/mock-draft-body'
import { exampleAccount } from '@/features/accounts/test-fixtures'
import { MailBodyEditor } from './test-harness'

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, convertFileSrc: () => 'mailx-body://localhost/' }))

beforeAll(() => {
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
})

const snapshot = (over: Partial<RunSnapshot> = {}): RunSnapshot => ({
  runId: 'r1', kind: 'composeAssist', state: 'running', text: '', tools: [], reasoning: [], toolsAvailable: false,
  error: null, finishedReason: null, completedAt: null, seq: 1, conversationId: null, pending: null, ...over,
})

function draft(bodyHtml: string, kind: DraftKind = 'new'): Draft {
  return {
    id: 'd', accountId: 'a', kind, origin: 'user', sourceEmailId: null, to: [], cc: [], bcc: [], subject: '',
    ...mockDraftBody(bodyHtml), inReplyTo: null, references: null,
    createdAt: 't', updatedAt: 't', revision: 1, attachments: [],
  }
}

async function setup({ bodyHtml = '<p></p>', kind = 'new' as DraftKind, usable = true } = {}) {
  const client = testClient([exampleAccount()])
  client.composeAssistState.mockResolvedValue({ usable, consented: true, service: 'OpenAI' })
  let send: (event: unknown) => void = () => undefined
  client.onAssistantEvent.mockImplementation((listener) => { send = listener as never; return () => undefined })
  const started: ComposeTask[] = []
  client.composeAssistStart.mockImplementation(async (_draftId, task) => { started.push(task); return snapshot() })
  const handle: { current: MailBodyEditorHandle | null } = { current: null }
  const view = render(<MailBodyEditor client={client} draft={draft(bodyHtml, kind)} handleRef={handle} />)
  /** Like the host: the run is under way before its events arrive. */
  const emit = async (...kinds: { runId: string; seq: number; kind: unknown }[]) => {
    await waitFor(() => expect(client.composeAssistStart).toHaveBeenCalled())
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20))
      for (const kind of kinds) send(kind)
    })
  }
  const body = await screen.findByRole('textbox', { name: '正文' }, { timeout: 5000 })
  if (usable) await screen.findByRole('button', { name: 'AI 写作' }, { timeout: 5000 })
  return { client, started, emit, body, view, handle }
}
const text = (delta: string, seq = 2) => ({ runId: 'r1', seq, kind: { type: 'text', delta } })
const done = (reason = 'stop') => ({ runId: 'r1', seq: 3, kind: { type: 'finished', reason } })
const completed = { runId: 'r1', seq: 4, kind: { type: 'state', state: 'completed' } }
/** The card as it is now: its DOM changes as the run moves between states. */
const cardRegion = () => screen.getByRole('region', { name: /起草|改写|校对|AI 写作/ })
describe('writing help', () => {
  it('offers nothing when the assistant cannot help', async () => {
    await setup({ usable: false })
    await waitFor(() => expect(screen.queryByRole('button', { name: 'AI 写作' })).not.toBeInTheDocument())
  })

  it('drafts from the empty body with Cmd/Ctrl+J and takes the answer in one undoable step', async () => {
    const { client, started, emit, body } = await setup()
    fireEvent.click(body)
    await waitFor(() => expect(screen.getByRole('button', { name: /让 AI 起草/ })).toBeInTheDocument())
    fireEvent.keyDown(body, { code: 'KeyJ', key: 'j', metaKey: true })
    const prompt = await screen.findByRole('textbox', { name: '起草要求' })
    fireEvent.change(prompt, { target: { value: '婉拒，下周再约' } })
    fireEvent.submit(prompt.closest('form')!)
    await waitFor(() => expect(started).toEqual([{ type: 'draft', instruction: '婉拒，下周再约' }]))
    expect(client.composeAssistStart).toHaveBeenCalledWith('d', { type: 'draft', instruction: '婉拒，下周再约' })
    await screen.findByRole('region', { name: '起草' })
    await emit(text('你好，'), text('下周再约。', 3), { ...done(), seq: 4 }, { ...completed, seq: 5 })
    await waitFor(() => expect(cardRegion()).toHaveTextContent('下周再约。'))
    fireEvent.keyDown(cardRegion(), { key: 'Enter' })
    await waitFor(() => expect(body).toHaveTextContent('下周再约。'))
    expect(screen.queryByRole('region', { name: '起草' })).not.toBeInTheDocument()
  })

  it('opens the selection menu over the text with Cmd/Ctrl+J', async () => {
    const { body, handle } = await setup({ bodyHtml: '<p>今年可能不行</p><p>保持原样</p>' })
    await act(async () => { handle.current?.setSelection({ from: 1, to: 7 }) })
    fireEvent.keyDown(body, { code: 'KeyJ', key: 'j', metaKey: true })
    const bubble = await screen.findByRole('toolbar', { name: 'AI 写作工具' })
    expect(within(bubble).getByRole('button', { name: /改写/ })).toBeInTheDocument()
    expect(within(bubble).getByRole('button', { name: '校对' })).toBeInTheDocument()
  })

  it('rewrites the selection and only the answer replaces it', async () => {
    const { started, emit, body, handle } = await setup({ bodyHtml: '<p>今年可能不行</p><p>保持原样</p>' })
    // The same window the selection menu opens: its items are covered by the toolbar menu here.
    await act(async () => { handle.current?.setSelection({ from: 1, to: 7 }) })
    fireEvent.click(screen.getByRole('button', { name: 'AI 写作' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: '更正式' }))
    await waitFor(() => expect(started).toEqual([{ type: 'rewrite', text: '今年可能不行', style: { type: 'formal' } }]))
    await screen.findByRole('region', { name: '改写 · 更正式' })
    await emit(text('今年恐怕不便'), done(), completed)
    await waitFor(() => expect(cardRegion()).toHaveTextContent('今年恐怕不便'))
    // The text being rewritten is dimmed while the card is open.
    await waitFor(() => expect(body.querySelector('.mail-ai-source')).toHaveTextContent('今年可能不行'))
    fireEvent.click(within(cardRegion()).getByRole('button', { name: /采用/ }))
    await waitFor(() => expect(body.querySelectorAll('p')[0]).toHaveTextContent('今年恐怕不便'))
    expect(body.querySelectorAll('p')[1]).toHaveTextContent('保持原样')
    expect(body.querySelector('.mail-ai-source')).toBeNull()
  })

  it('a proofread shows its changes and sends nothing with them', async () => {
    const { started, emit, body } = await setup({ bodyHtml: '<p>teh meeting</p>' })
    fireEvent.click(screen.getByRole('button', { name: 'AI 写作' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: '校对' }))
    await waitFor(() => expect(started).toEqual([{ type: 'proofread', text: 'teh meeting' }]))
    await screen.findByRole('region', { name: '校对' })
    await emit(text('the meeting'), done(), completed)
    await waitFor(() => expect(within(cardRegion()).getByLabelText(/处修改/)).toBeInTheDocument())
    expect(cardRegion().querySelector('del')).toHaveTextContent('teh')
    expect(cardRegion().querySelector('ins')).toHaveTextContent('the')
    fireEvent.click(within(cardRegion()).getByRole('button', { name: /采用/ }))
    await waitFor(() => expect(body.querySelector('p')).toHaveTextContent('the meeting'))
  })

  it('never rewrites the quoted original of a reply', async () => {
    const { client, started, handle } = await setup({ bodyHtml: '<p></p><p>On Monday, Chen wrote:</p><blockquote><p>original text</p></blockquote>', kind: 'reply' })
    // A selection inside the quoted original (the "On Monday…" line and the quote): never sent.
    await act(async () => { handle.current?.setSelection({ from: 3, to: 18 }) })
    fireEvent.click(screen.getByRole('button', { name: 'AI 写作' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: '校对' }))
    await waitFor(() => expect(screen.getByRole('region', { name: 'AI 写作' })).toHaveTextContent('引用的原邮件不会送去改写'))
    expect(started).toEqual([])
    expect(client.composeAssistStart).not.toHaveBeenCalled()
  })

  it('asks for consent in the card and starts again once given', async () => {
    const { client, emit } = await setup({ bodyHtml: '<p>text</p>' })
    client.composeAssistGrantConsent.mockResolvedValue({ connectionId: 'c1', baseUrl: 'https://api.example.test/v1' })
    fireEvent.click(screen.getByRole('button', { name: 'AI 写作' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: '校对' }))
    await screen.findByRole('region', { name: '校对' })
    await emit({ runId: 'r1', seq: 2, kind: { type: 'state', state: 'awaitingConsent' } })
    await waitFor(() => expect(cardRegion()).toHaveTextContent('OpenAI'))
    fireEvent.click(within(cardRegion()).getByRole('button', { name: '允许' }))
    fireEvent.click(await screen.findByRole('button', { name: '同意' }))
    await waitFor(() => expect(client.composeAssistGrantConsent).toHaveBeenCalled())
    await waitFor(() => expect(client.composeAssistStart).toHaveBeenCalledTimes(2))
  })

  it('drops the card when the text it is about is edited', async () => {
    const { emit, body } = await setup({ bodyHtml: '<p>text</p>' })
    fireEvent.click(screen.getByRole('button', { name: 'AI 写作' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: '校对' }))
    await screen.findByRole('region', { name: '校对' })
    await emit(text('the text'), done(), completed)
    await waitFor(() => expect(screen.getByRole('region', { name: '校对' })).toBeInTheDocument())
    // Type inside the rewritten text: the answer no longer describes it.
    fireEvent.click(body.querySelector('p')!)
    fireEvent.keyDown(body, { key: 'End' })
    fireEvent.keyPress(body, { key: '!' })
    fireEvent.input(body, { target: { textContent: 'text!' }, inputType: 'insertText' })
    await waitFor(() => expect(screen.queryByRole('region', { name: '校对' })).not.toBeInTheDocument())
  })

  it('drops section tags a model copied from its input, while streaming too', async () => {
    const { started, emit, body } = await setup()
    fireEvent.click(body)
    fireEvent.keyDown(body, { code: 'KeyJ', key: 'j', metaKey: true })
    const prompt = await screen.findByRole('textbox', { name: '起草要求' })
    fireEvent.change(prompt, { target: { value: '问好' } })
    fireEvent.submit(prompt.closest('form')!)
    await waitFor(() => expect(started).toHaveLength(1))
    await emit(text('<draft>\n你好，'), text('收到。\n</draft>', 3), { ...done(), seq: 4 }, { ...completed, seq: 5 })
    await waitFor(() => expect(cardRegion()).toHaveTextContent('收到。'))
    expect(cardRegion()).not.toHaveTextContent('<draft>')
    expect(cardRegion()).not.toHaveTextContent('</draft>')
    fireEvent.keyDown(cardRegion(), { key: 'Enter' })
    await waitFor(() => expect(body).toHaveTextContent('收到。'))
    expect(body.textContent).not.toContain('draft')
  })

  it('keeps the paragraphs a model wrote with single line breaks', async () => {
    const { started, emit, body } = await setup()
    fireEvent.click(body)
    fireEvent.keyDown(body, { code: 'KeyJ', key: 'j', metaKey: true })
    const prompt = await screen.findByRole('textbox', { name: '起草要求' })
    fireEvent.change(prompt, { target: { value: '问好' } })
    fireEvent.submit(prompt.closest('form')!)
    await waitFor(() => expect(started).toHaveLength(1))
    await emit(text('你好！\n好久不见了。'), text('\n祝好\nGuqier', 3), { ...done(), seq: 4 }, { ...completed, seq: 5 })
    await waitFor(() => expect(cardRegion()).toHaveTextContent('好久不见了。'))
    fireEvent.keyDown(cardRegion(), { key: 'Enter' })
    await waitFor(() => expect(body.textContent).toContain('好久不见了。'))
    const paragraphs = [...body.querySelectorAll('p')].map((p) => p.textContent)
    expect(paragraphs).toEqual(['你好！', '好久不见了。', '祝好', 'Guqier'])
  })

  it('shows exactly the HTML it inserts: the preview is the inserted content', async () => {
    const { started, emit, body } = await setup()
    fireEvent.click(body)
    fireEvent.keyDown(body, { code: 'KeyJ', key: 'j', metaKey: true })
    const prompt = await screen.findByRole('textbox', { name: '起草要求' })
    fireEvent.change(prompt, { target: { value: '问好' } })
    fireEvent.submit(prompt.closest('form')!)
    await waitFor(() => expect(started).toHaveLength(1))
    await emit(text('你好！\n好久不见了。\n- 第一件事\n- 第二件事'), { ...done(), seq: 3 }, { ...completed, seq: 4 })
    await waitFor(() => expect(cardRegion().querySelector('.mail-ai-preview')).not.toBeNull())
    const preview = cardRegion().querySelector('.mail-ai-preview')!
    const before = { paragraphs: preview.querySelectorAll('p').length, items: preview.querySelectorAll('li').length }
    fireEvent.keyDown(cardRegion(), { key: 'Enter' })
    await waitFor(() => expect(body.textContent).toContain('第二件事'))
    expect({ paragraphs: body.querySelectorAll('p').length, items: body.querySelectorAll('li').length }).toEqual(before)
    expect([...body.querySelectorAll('p')].map((p) => p.textContent)).toEqual(['你好！', '好久不见了。', '第一件事', '第二件事'])
  })

  it('keeps the blank lines a model wrote as the space between paragraphs', async () => {
    const { started, emit, body } = await setup()
    fireEvent.click(body)
    fireEvent.keyDown(body, { code: 'KeyJ', key: 'j', metaKey: true })
    const prompt = await screen.findByRole('textbox', { name: '起草要求' })
    fireEvent.change(prompt, { target: { value: '问好' } })
    fireEvent.submit(prompt.closest('form')!)
    await waitFor(() => expect(started).toHaveLength(1))
    await emit(text('你好！\n\n好久不见了。\n\n祝好\nGuqier'), { ...done(), seq: 3 }, { ...completed, seq: 4 })
    await waitFor(() => expect(cardRegion().querySelector('.mail-ai-preview')).not.toBeNull())
    // The blank lines are there before taking, in the same shape the draft will get.
    expect([...cardRegion().querySelectorAll('p')].filter((p) => !p.textContent)).toHaveLength(2)
    fireEvent.keyDown(cardRegion(), { key: 'Enter' })
    await waitFor(() => expect(body.textContent).toContain('Guqier'))
    expect([...body.querySelectorAll('p')].map((p) => p.textContent)).toEqual(['你好！', '', '好久不见了。', '', '祝好', 'Guqier'])
  })
})
