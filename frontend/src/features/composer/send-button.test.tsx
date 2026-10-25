import { i18n } from '@/i18n'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CONFIRM_GUARD_MS, CONFIRM_TIMEOUT_MS, SendButton } from './send-button'

const matchMedia = window.matchMedia

function setup(props: Partial<Parameters<typeof SendButton>[0]> = {}) {
  const onPrepare = vi.fn(async () => 7 as number | null)
  const onConfirm = vi.fn(async () => undefined)
  const view = render(<SendButton subjectEmpty={false} resetToken="a" onPrepare={onPrepare} onConfirm={onConfirm} {...props} />)
  const button = screen.getByRole('button')
  const press = async () => { await act(async () => { fireEvent.click(button) }) }
  const pressKey = async (init: KeyboardEventInit = { metaKey: true }) => { await act(async () => { fireEvent.keyDown(window, { key: 'Enter', ...init }) }) }
  return { ...view, button, press, pressKey, onPrepare, onConfirm }
}

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  vi.useRealTimers()
  window.matchMedia = matchMedia
  void i18n.changeLanguage('zh-CN')
})

describe('two-step send button', () => {
  it('first press validates and asks again; the second press queues at the saved revision', async () => {
    const { button, press, onPrepare, onConfirm } = setup()
    expect(button).toHaveTextContent('发送')
    await press()
    expect(onPrepare).toHaveBeenCalledTimes(1)
    expect(button).toHaveTextContent('确认发送')
    expect(button).toHaveAttribute('data-stage', 'confirm')
    expect(screen.getByRole('status')).toHaveTextContent('再次按下以确认发送')
    expect(onConfirm).not.toHaveBeenCalled()

    await act(async () => { vi.advanceTimersByTime(CONFIRM_GUARD_MS + 1) })
    await press()
    expect(onConfirm).toHaveBeenCalledWith(7)
    expect(button).toHaveTextContent('发送')
  })

  it('stays put when the check refuses', async () => {
    const { button, press, onConfirm } = setup({ onPrepare: vi.fn(async () => null) })
    await press()
    expect(button).toHaveAttribute('data-stage', 'idle')
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('ignores a click inside the guard interval', async () => {
    const { button, press, onConfirm } = setup()
    await press()
    await act(async () => { vi.advanceTimersByTime(CONFIRM_GUARD_MS - 50) })
    await press()
    expect(onConfirm).not.toHaveBeenCalled()
    expect(button).toHaveAttribute('data-stage', 'confirm')
    await act(async () => { vi.advanceTimersByTime(60) })
    await press()
    expect(onConfirm).toHaveBeenCalledTimes(1)
  })

  it('shows the no-subject label when the subject is empty', async () => {
    const { button, press } = setup({ subjectEmpty: true })
    await press()
    expect(button).toHaveTextContent('无主题，确认发送')
  })

  it('reverts after about four seconds without a second press', async () => {
    const { button, press } = setup()
    await press()
    await act(async () => { vi.advanceTimersByTime(CONFIRM_TIMEOUT_MS - 10) })
    expect(button).toHaveAttribute('data-stage', 'confirm')
    await act(async () => { vi.advanceTimersByTime(20) })
    expect(button).toHaveAttribute('data-stage', 'idle')
    expect(screen.getByRole('status')).toHaveTextContent('')
  })

  it('updates the confirmation text without restarting its timer or sending when the language changes', async () => {
    const { button, press, onPrepare, onConfirm } = setup()
    await press()
    await act(async () => { vi.advanceTimersByTime(CONFIRM_TIMEOUT_MS / 2) })
    await act(async () => { await i18n.changeLanguage('en') })
    expect(button).toHaveTextContent('Confirm send')
    expect(button).toHaveAttribute('data-stage', 'confirm')
    expect(onPrepare).toHaveBeenCalledTimes(1)
    expect(onConfirm).not.toHaveBeenCalled()
    await act(async () => { vi.advanceTimersByTime(CONFIRM_TIMEOUT_MS / 2 + 1) })
    expect(button).toHaveAttribute('data-stage', 'idle')
    expect(button).toHaveTextContent('Send')
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('reverts on any edit, on Escape and when the window loses focus', async () => {
    const { button, press, rerender, onPrepare, onConfirm } = setup()
    const props = { subjectEmpty: false, onPrepare, onConfirm }
    await press()
    rerender(<SendButton {...props} resetToken="b" />)
    expect(button).toHaveAttribute('data-stage', 'idle')

    await press()
    expect(button).toHaveAttribute('data-stage', 'confirm')
    await act(async () => { fireEvent.keyDown(window, { key: 'Escape' }) })
    expect(button).toHaveAttribute('data-stage', 'idle')

    await press()
    expect(button).toHaveAttribute('data-stage', 'confirm')
    await act(async () => { fireEvent.blur(window) })
    expect(button).toHaveAttribute('data-stage', 'idle')
  })

  it('needs Cmd/Ctrl+Enter twice, with the same guard', async () => {
    const { button, pressKey, onConfirm } = setup()
    await pressKey({ ctrlKey: true })
    expect(button).toHaveAttribute('data-stage', 'confirm')
    await pressKey({ ctrlKey: true })
    expect(onConfirm).not.toHaveBeenCalled()
    await act(async () => { vi.advanceTimersByTime(CONFIRM_GUARD_MS + 1) })
    await pressKey({ metaKey: true })
    expect(onConfirm).toHaveBeenCalledWith(7)
  })

  it('plays no transition under reduced motion but keeps the guard', async () => {
    window.matchMedia = vi.fn().mockImplementation((query: string) => ({
      matches: query.includes('reduce'), media: query, addEventListener: vi.fn(), removeEventListener: vi.fn(),
    }))
    const { button, press, onConfirm } = setup()
    expect(button.className).not.toMatch(/transition-\[width/)
    await press()
    await press()
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('animates width and color (about 200 ms) otherwise', () => {
    const { button } = setup()
    expect(button.className).toMatch(/transition-\[width,background-color,color\]/)
    expect(button.className).toMatch(/duration-200/)
  })
})
