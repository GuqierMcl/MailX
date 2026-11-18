import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, within } from '@/test/render'
import { Markdown } from './markdown-view'

const clipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard')

afterEach(() => {
  vi.useRealTimers()
  if (clipboard) Object.defineProperty(navigator, 'clipboard', clipboard)
  else Reflect.deleteProperty(navigator, 'clipboard')
})

const view = (text: string, streaming = false, onOpenLink = vi.fn()) =>
  <Markdown text={text} streaming={streaming} onOpenLink={onOpenLink} />

describe('Markdown view', () => {
  it('draws a table with its header, alignment and no unsafe link', () => {
    render(view('| Name | Amount |\n|:--|--:|\n| **Bill** | 1,200 |\n| [x](javascript:alert(1)) | 3 |'))
    const table = screen.getByRole('table')
    expect(within(table).getAllByRole('columnheader').map((cell) => cell.textContent)).toEqual(['Name', 'Amount'])
    const cells = within(table).getAllByRole('cell')
    expect(cells[1]).toHaveStyle({ textAlign: 'right' })
    expect(cells[0].querySelector('strong')).toHaveTextContent('Bill')
    expect(cells[2]).toHaveTextContent('x')
    expect(within(table).queryByRole('link')).not.toBeInTheDocument()
    // It scrolls within the column rather than widening the reply.
    expect(table.parentElement).toHaveClass('overflow-x-auto')
  })

  it('never inserts HTML or loads an image', () => {
    const { container } = render(view('<img src=x onerror=alert(1)>\n\nhi <b>x</b> ![p](https://example.com/p.png)'))
    expect(container.querySelector('img, b, script')).toBeNull()
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument()
    expect(screen.getByText('hi <b>x</b> p')).toBeInTheDocument()
  })

  it('shows task items with a static mark, strikethrough, and a short rule', () => {
    render(view('- [x] paid\n- [ ] reply\n\n~~old~~\n\n---\n\nend'))
    expect(screen.getByRole('img', { name: '已完成' })).toBeInTheDocument()
    expect(screen.getByRole('img', { name: '未完成' })).toBeInTheDocument()
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument()
    expect(screen.getByText('old').tagName).toBe('DEL')
    expect(screen.getByRole('separator')).toBeInTheDocument()
  })

  it('labels a code block with its language and copies the code', async () => {
    vi.useFakeTimers()
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    render(view('```ts\nconst a = 1\n```'))
    expect(screen.getByText('ts')).toBeInTheDocument()
    expect(screen.getByText('const a = 1').closest('pre')).toBeInTheDocument()
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: '复制代码' })) })
    expect(writeText).toHaveBeenCalledWith('const a = 1')
    expect(screen.getByRole('button', { name: '已复制' })).toBeInTheDocument()
    act(() => { vi.advanceTimersByTime(1600) })
    expect(screen.getByRole('button', { name: '复制代码' })).toBeInTheDocument()
  })

  it('opens a link through the handler, not in the page', () => {
    const open = vi.fn()
    render(view('see [docs](https://example.com/docs)', false, open))
    fireEvent.click(screen.getByRole('link', { name: 'docs' }))
    expect(open).toHaveBeenCalledWith('https://example.com/docs')
  })

  it('shows the caret only while streaming, at the end of the text, then fades it out', () => {
    vi.useFakeTimers()
    const { container, rerender } = render(view('Hello **wor', true))
    const caret = container.querySelector('[data-slot="stream-caret"]')
    expect(caret).toHaveAttribute('data-state', 'live')
    expect(caret?.closest('p')).toHaveTextContent('Hello wor')
    expect(container.querySelector('strong')).toHaveTextContent('wor')

    // After a code block the caret takes its own line.
    rerender(view('Hello **world**\n\n```\nx\n```', true))
    expect(container.querySelector('[data-slot="stream-caret"]')?.closest('p, pre')).toBeNull()

    rerender(view('Hello **world**\n\n```\nx\n```', false))
    expect(container.querySelector('[data-slot="stream-caret"]')).toHaveAttribute('data-state', 'leaving')
    act(() => { vi.advanceTimersByTime(250) })
    expect(container.querySelector('[data-slot="stream-caret"]')).toBeNull()
  })

  it('has no caret and no entrance for a finished reply', () => {
    const { container } = render(view('one\n\ntwo'))
    expect(container.querySelector('[data-slot="stream-caret"]')).toBeNull()
    expect(container.querySelector('.animate-md-block-in')).toBeNull()
  })

  it('fades in only blocks that arrive while streaming, once', () => {
    const { rerender } = render(view('first', true))
    expect(screen.getByText('first')).not.toHaveClass('animate-md-block-in')
    rerender(view('first\n\nsecond', true))
    const second = screen.getByText('second')
    expect(second).toHaveClass('animate-md-block-in')
    rerender(view('first\n\nsecond grows', true))
    // The same element keeps growing: the entrance does not start again.
    expect(screen.getByText('second grows')).toBe(second)
  })
})
