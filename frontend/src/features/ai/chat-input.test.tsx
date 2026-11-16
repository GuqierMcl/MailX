import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { fireEvent, render, screen } from '@/test/render'
import { ChatInput } from './chat-input'

function setup(working = false) {
  const handlers = { onSend: vi.fn(), onStop: vi.fn(), onEscape: vi.fn() }
  const view = render(<ChatInput working={working} {...handlers} />)
  return { ...handlers, field: screen.getByRole('textbox', { name: '问问你的邮件' }) as HTMLTextAreaElement, ...view }
}

describe('ChatInput', () => {
  it('sends the trimmed text on Enter and clears the field', async () => {
    const { field, onSend } = setup()
    await userEvent.type(field, '  你好  {Enter}')
    expect(onSend).toHaveBeenCalledWith('你好')
    expect(field).toHaveValue('')
  })

  it('breaks the line on Shift+Enter and does not send', async () => {
    const { field, onSend } = setup()
    await userEvent.type(field, '一{Shift>}{Enter}{/Shift}二')
    expect(field).toHaveValue('一\n二')
    expect(onSend).not.toHaveBeenCalled()
  })

  it('never sends while the input method is composing', async () => {
    const { field, onSend } = setup()
    await userEvent.type(field, 'ni')
    fireEvent.keyDown(field, { key: 'Enter', isComposing: true })
    fireEvent.keyDown(field, { key: 'Enter', keyCode: 229 })
    expect(onSend).not.toHaveBeenCalled()
    expect(field).toHaveValue('ni')
  })

  it('does not send an empty message', async () => {
    const { field, onSend } = setup()
    await userEvent.type(field, '   {Enter}')
    expect(onSend).not.toHaveBeenCalled()
  })

  it('sends with the round button too, which is 停止 while a reply is going', async () => {
    const { field, onSend, onStop, rerender } = setup()
    await userEvent.type(field, '好')
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(onSend).toHaveBeenCalledWith('好')
    rerender(<ChatInput working onSend={onSend} onStop={onStop} onEscape={vi.fn()} />)
    await userEvent.click(screen.getByRole('button', { name: '停止' }))
    expect(onStop).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole('button', { name: '发送' })).not.toBeInTheDocument()
  })

  it('ignores typing while a reply is going, and flows the border only then', async () => {
    const { field, onSend, container } = setup(true)
    expect(field).toHaveAttribute('readonly')
    await userEvent.type(field, '不行{Enter}')
    expect(field).toHaveValue('')
    expect(onSend).not.toHaveBeenCalled()
    expect(container.querySelector('[data-slot="ai-border"]')).toHaveAttribute('data-flow', 'true')
  })

  it('keeps the border still when idle', () => {
    const { container } = setup()
    expect(container.querySelector('[data-slot="ai-border"]')).toHaveAttribute('data-flow', 'false')
  })

  it('Esc stops a running reply, otherwise leaves', async () => {
    const running = setup(true)
    await userEvent.type(running.field, '{Escape}')
    expect(running.onStop).toHaveBeenCalledTimes(1)
    expect(running.onEscape).not.toHaveBeenCalled()
    running.unmount()
    const idle = setup()
    await userEvent.type(idle.field, '{Escape}')
    expect(idle.onEscape).toHaveBeenCalledTimes(1)
    expect(idle.onStop).not.toHaveBeenCalled()
  })

  it('takes back words handed to it', () => {
    const { rerender, field, onSend, onStop, onEscape } = setup()
    rerender(<ChatInput working={false} onSend={onSend} onStop={onStop} onEscape={onEscape} restore={{ text: '没发出去的话', key: 1 }} />)
    expect(field).toHaveValue('没发出去的话')
  })
})
