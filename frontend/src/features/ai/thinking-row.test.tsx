import { afterEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen } from '@/test/render'
import { TooltipProvider } from '@/components/ui/tooltip'
import { testClient } from '@/features/accounts/test-fixtures'
import type { ConversationDetail } from '@/generated/accounts'
import { ChatEnvContext, type ChatEnv } from './chat-env'
import { ChatMessages, SLOW_REPLY_MS } from './chat-messages'
import { savedMessages, turnFragments, type ChatMessage, type ChatTurn } from './chat-model'
import { ThinkingRow } from './thinking-row'
import type { RunThought, RunTool } from './use-assistant-run'

const thought = (over: Partial<RunThought> = {}): RunThought => ({
  segment: 0, text: '先看看未读邮件。', at: 0, toolsBefore: 0, startedAt: Date.now(), durationMs: null, ...over,
})
const tool = (callId: string, at = 0): RunTool => ({ callId, name: 'search_emails', status: 'done', summary: { type: 'found', count: 2 }, at, settledAt: null })
const turn = (over: Partial<ChatTurn>): ChatTurn => ({
  state: 'running', text: '', tools: [], confirms: [], answers: {}, pendingId: null, error: null, finishedReason: null, finishedAt: null, ...over,
})
const reply = (over: Partial<ChatTurn>): ChatMessage => ({ id: 'r', role: 'assistant', turn: turn(over) })

function view(children: React.ReactNode) {
  const env: ChatEnv = {
    client: testClient(), accounts: [], folders: [], onOpenFolder: vi.fn(), undone: new Set(),
    onOpenEmail: vi.fn(), onOpenDraft: vi.fn(), respond: vi.fn(), retry: vi.fn(), focusInput: vi.fn(), markUndone: vi.fn(),
  }
  return <TooltipProvider><ChatEnvContext value={env}>{children}</ChatEnvContext></TooltipProvider>
}

const reduceMotion = (reduce: boolean) => {
  vi.mocked(window.matchMedia).mockImplementation((query: string) => ({
    matches: reduce && query.includes('reduced-motion'), media: query, onchange: null,
    addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
  }))
}

afterEach(() => { vi.useRealTimers(); reduceMotion(false) })

describe('ThinkingRow', () => {
  it('counts the seconds while it thinks, with no reasoning text and no toggle', () => {
    vi.useFakeTimers()
    const { rerender } = render(view(<ThinkingRow thought={thought()} working />))
    expect(screen.getByText('思考中')).toHaveClass('ai-shimmer')
    expect(screen.queryByText('先看看未读邮件。')).not.toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
    act(() => { vi.advanceTimersByTime(12_000) })
    expect(screen.getByText(/12 秒/)).toBeInTheDocument()
    // Done: the words change in place and the line becomes a closed toggle.
    rerender(view(<ThinkingRow thought={thought({ durationMs: 12_400 })} working />))
    const toggle = screen.getByRole('button', { name: /思考了 12 秒/ })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByText('思考中')).not.toBeInTheDocument()
  })

  it('opens and closes the raw reasoning, collapsed at first', async () => {
    render(view(<ThinkingRow thought={thought({ text: '第一步\n  第二步', durationMs: 3000 })} working={false} />))
    const toggle = screen.getByRole('button', { name: /思考了 3 秒/ })
    const panel = document.getElementById(toggle.getAttribute('aria-controls')!)!
    expect(panel).toHaveAttribute('data-open', 'false')
    expect(panel).toHaveAttribute('inert')
    await userEvent.click(toggle)
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    expect(panel).toHaveAttribute('data-open', 'true')
    expect(panel).not.toHaveAttribute('inert')
    const text = panel.querySelector('p')!
    expect(text.textContent).toBe('第一步\n  第二步')
    expect(text).toHaveClass('whitespace-pre-wrap', 'border-l-2')
    expect(panel.className).toMatch(/transition-\[grid-template-rows,opacity\]/)
    await userEvent.click(toggle)
    expect(panel).toHaveAttribute('data-open', 'false')
  })

  it('says 思考了片刻 under a second, and has nothing to open for redacted reasoning', () => {
    const { rerender } = render(view(<ThinkingRow thought={thought({ durationMs: 400 })} working={false} />))
    expect(screen.getByRole('button', { name: /思考了片刻/ })).toBeInTheDocument()
    rerender(view(<ThinkingRow thought={thought({ text: '', durationMs: 5000 })} working={false} />))
    expect(screen.getByText('思考了 5 秒')).toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('opens without a transition under reduced motion', () => {
    reduceMotion(true)
    render(view(<ThinkingRow thought={thought({ durationMs: 2000 })} working={false} />))
    const panel = document.querySelector('[data-slot="thinking-panel"]')!
    expect(panel).toHaveClass('transition-none')
    expect(panel.className).not.toMatch(/duration-200/)
  })
})

describe('thinking in a reply', () => {
  it('shows the generic line and the slow note only before anything arrives', () => {
    vi.useFakeTimers()
    const { rerender } = render(view(<ChatMessages messages={[reply({ state: 'running' })]} />))
    expect(screen.getByText('正在思考…')).toBeInTheDocument()
    act(() => { vi.advanceTimersByTime(SLOW_REPLY_MS + 10) })
    expect(screen.getByText('仍在等待模型响应')).toBeInTheDocument()
    // Reasoning arrives: one indicator only, and no slow note while it streams.
    rerender(view(<ChatMessages messages={[reply({ state: 'running', reasoning: [thought()] })]} />))
    expect(screen.queryByText('正在思考…')).not.toBeInTheDocument()
    expect(screen.queryByText('仍在等待模型响应')).not.toBeInTheDocument()
    expect(screen.getByText('思考中')).toBeInTheDocument()
    act(() => { vi.advanceTimersByTime(SLOW_REPLY_MS * 3) })
    expect(screen.queryByText('仍在等待模型响应')).not.toBeInTheDocument()
  })

  it('never shows the slow note once reasoning is streaming from the start', () => {
    vi.useFakeTimers()
    render(view(<ChatMessages messages={[reply({ state: 'running', reasoning: [thought()] })]} />))
    act(() => { vi.advanceTimersByTime(SLOW_REPLY_MS * 2) })
    expect(screen.queryByText('仍在等待模型响应')).not.toBeInTheDocument()
    expect(screen.queryByText('正在思考…')).not.toBeInTheDocument()
  })

  it('gives each stretch its own line, before and after a tool call', () => {
    const fragments = turnFragments(turn({
      state: 'completed', text: 'Hello world',
      tools: [tool('a', 5)],
      reasoning: [thought({ segment: 0, at: 0, toolsBefore: 0, durationMs: 1000 }), thought({ segment: 1, at: 5, toolsBefore: 1, durationMs: 2000 })],
    }))
    expect(fragments.map((part) => part.kind === 'text' ? `text:${part.text}` : part.kind === 'thought' ? `thought:${part.thought.segment}` : part.kind)).toEqual([
      'thought:0', 'text:Hello', 'tool', 'thought:1', 'text: world',
    ])
    // A stretch after a tool at the same offset goes after it; one before goes before.
    const same = turnFragments(turn({
      tools: [tool('a'), tool('b')],
      reasoning: [thought({ segment: 0, toolsBefore: 0 }), thought({ segment: 1, toolsBefore: 1 }), thought({ segment: 2, toolsBefore: 2 })],
    }))
    expect(same.map((part) => part.kind === 'thought' ? `t${part.thought.segment}` : part.kind === 'tool' ? part.tool.callId : part.kind)).toEqual(['t0', 'a', 't1', 'b', 't2'])
  })

  it('restores the stretches of a saved reply in place, collapsed', async () => {
    const detail: ConversationDetail = {
      id: 'c', title: 't', createdAt: '2030-01-01T00:00:00Z', updatedAt: '2030-01-01T00:00:00Z',
      turns: [
        { role: 'user', text: '查一下', at: '2030-01-01T00:00:00Z' },
        {
          role: 'assistant', text: '先看看。结果如下。', state: 'completed', finishedReason: 'stop', error: null, at: '2030-01-01T00:00:00Z', confirmations: [],
          tools: [{ callId: 'a', name: 'search_emails', status: 'done', summary: { type: 'found', count: 7 }, textOffset: 4 }],
          reasoning: [
            { text: '要先搜索。', durationMs: 4200, textOffset: 0, toolsBefore: 0 },
            { text: '', durationMs: 900, textOffset: 4, toolsBefore: 1 },
          ],
        },
      ],
    }
    const messages = savedMessages(detail)
    const saved = messages[1]
    if (saved.role !== 'assistant') throw new Error('expected a reply')
    expect(turnFragments(saved.turn).map((part) => part.kind)).toEqual(['thought', 'text', 'tool', 'thought', 'text'])
    render(view(<ChatMessages messages={messages} />))
    const toggle = screen.getByRole('button', { name: /思考了 4 秒/ })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    // Redacted (empty) reasoning has no toggle.
    expect(screen.getByText('思考了片刻').closest('button')).toBeNull()
    await userEvent.click(toggle)
    expect(screen.getByText('要先搜索。')).toBeVisible()
  })
})
