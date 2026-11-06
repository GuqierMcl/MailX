import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { withTitleBar } from '@/test/title-bar'
import { ShellLayout } from './shell-layout'
import { NAV_DEFAULT, NAV_MAX, NAV_MIN, NAV_RAIL } from './nav-width'

type Saved = { navWidth?: number; navCollapsed?: boolean }

/** Renders the shell with saved layout values; `save` re-renders with a change, as the settings query would. */
const setup = ({ navWidth = NAV_DEFAULT, navCollapsed = false }: Saved = {}) => {
  const onNavChange = vi.fn()
  const view = (saved: Required<Saved>) => withTitleBar(
    <ShellLayout navLabel="导航" nav={<p>菜单</p>} rail={<p>图标</p>} navWidth={saved.navWidth} navCollapsed={saved.navCollapsed} onNavChange={onNavChange}>
      <p>内容</p>
    </ShellLayout>,
  )
  const { rerender } = render(view({ navWidth, navCollapsed }))
  return {
    handle: screen.getByRole('separator', { name: '调整侧边栏宽度' }),
    onNavChange,
    save: (saved: Required<Saved>) => rerender(view(saved)),
  }
}
const width = () => Number(screen.getByRole('separator').getAttribute('aria-valuenow'))
const drag = (handle: HTMLElement, from: number, to: number) => {
  handle.setPointerCapture = () => undefined
  fireEvent.pointerDown(handle, { button: 0, clientX: from, pointerId: 1 })
  fireEvent.pointerMove(handle, { clientX: to, pointerId: 1 })
}

/** Makes the window narrow (or wide again) for the narrow-window query. */
function narrowWindow(narrow: boolean) {
  const listeners: (() => void)[] = []
  vi.mocked(window.matchMedia).mockImplementation((query: string) => ({
    matches: narrow, media: query, onchange: null,
    addListener: vi.fn(), removeListener: vi.fn(), dispatchEvent: vi.fn(),
    addEventListener: (_: string, listener: () => void) => listeners.push(listener),
    removeEventListener: vi.fn(),
  }) as unknown as MediaQueryList)
  return {
    set(next: boolean) {
      narrow = next
      act(() => listeners.forEach((listener) => listener()))
    },
  }
}

afterEach(() => narrowWindow(false))

describe('nav rail resizing', () => {
  it('shows the saved width', () => {
    setup({ navWidth: 300 })
    expect(width()).toBe(300)
  })

  it('saves a drag once, clamped, when it ends', () => {
    const { handle, onNavChange } = setup()
    drag(handle, 100, 5000)
    expect(width()).toBe(NAV_MAX)
    expect(onNavChange).not.toHaveBeenCalled()
    fireEvent.pointerUp(handle, { pointerId: 1 })
    expect(onNavChange).toHaveBeenCalledExactlyOnceWith({ navWidth: NAV_MAX })
  })

  it('adjusts with the keyboard and saves on key release', () => {
    const { handle, onNavChange } = setup()
    fireEvent.keyDown(handle, { key: 'ArrowRight' })
    expect(width()).toBe(NAV_DEFAULT + 8)
    fireEvent.keyUp(handle, { key: 'ArrowRight' })
    expect(onNavChange).toHaveBeenCalledExactlyOnceWith({ navWidth: NAV_DEFAULT + 8 })
    fireEvent.keyDown(handle, { key: 'Home' })
    expect(width()).toBe(NAV_MIN)
  })

  it('resets to the default on double click', () => {
    const { handle, onNavChange } = setup({ navWidth: 300 })
    fireEvent.doubleClick(handle)
    expect(width()).toBe(NAV_DEFAULT)
    expect(onNavChange).toHaveBeenCalledWith({ navWidth: NAV_DEFAULT })
  })
})

describe('icon rail', () => {
  it('folds to the rail instead of hiding, and saves the choice', () => {
    const { onNavChange } = setup()
    expect(screen.getByText('菜单')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '收起侧边栏' }))
    expect(screen.getByText('图标')).toBeInTheDocument()
    expect(screen.queryByText('菜单')).not.toBeInTheDocument()
    expect(width()).toBe(NAV_RAIL)
    expect(onNavChange).toHaveBeenCalledExactlyOnceWith({ navCollapsed: true })
  })

  it('opens folded when the saved choice says so, and saves a quick fold and unfold both', () => {
    const { onNavChange } = setup({ navCollapsed: true })
    expect(screen.getByText('图标')).toBeInTheDocument()
    fireEvent.keyDown(window, { key: 'b', metaKey: true })
    fireEvent.keyDown(window, { key: 'b', metaKey: true })
    expect(screen.getByText('图标')).toBeInTheDocument()
    expect(onNavChange.mock.calls).toEqual([[{ navCollapsed: false }], [{ navCollapsed: true }]])
  })

  it('folds when dragged below the snap point and opens again when the rail is dragged out', () => {
    const { handle, onNavChange, save } = setup()
    drag(handle, 300, 300 - (NAV_DEFAULT - 100))
    expect(screen.getByText('图标')).toBeInTheDocument()
    fireEvent.pointerUp(handle, { pointerId: 1 })
    // Folding keeps the width the sidebar opens at again.
    expect(onNavChange).toHaveBeenCalledExactlyOnceWith({ navCollapsed: true })
    save({ navWidth: NAV_DEFAULT, navCollapsed: true })

    onNavChange.mockClear()
    drag(handle, 50, 50 + 200)
    expect(screen.getByText('菜单')).toBeInTheDocument()
    fireEvent.pointerUp(handle, { pointerId: 1 })
    expect(onNavChange.mock.calls).toEqual([[{ navCollapsed: false }], [{ navWidth: NAV_RAIL + 200 }]])
  })

  it('folds with the keyboard past the narrowest width and opens with ArrowRight', () => {
    const { handle, onNavChange } = setup({ navWidth: NAV_MIN })
    fireEvent.keyDown(handle, { key: 'ArrowLeft' })
    fireEvent.keyUp(handle, { key: 'ArrowLeft' })
    expect(screen.getByText('图标')).toBeInTheDocument()
    expect(onNavChange).toHaveBeenCalledExactlyOnceWith({ navCollapsed: true })
    fireEvent.keyDown(handle, { key: 'ArrowRight' })
    fireEvent.keyUp(handle, { key: 'ArrowRight' })
    expect(screen.getByText('菜单')).toBeInTheDocument()
  })

  it('shows the rail in a narrow window without changing the saved choice', () => {
    const window = narrowWindow(true)
    const { onNavChange } = setup()
    expect(screen.getByText('图标')).toBeInTheDocument()
    // Opening it there lasts until the window widens, and is not saved.
    fireEvent.click(screen.getByRole('button', { name: '展开侧边栏' }))
    expect(screen.getByText('菜单')).toBeInTheDocument()
    expect(onNavChange).not.toHaveBeenCalled()
    window.set(false)
    expect(screen.getByText('菜单')).toBeInTheDocument()
    window.set(true)
    expect(screen.getByText('图标')).toBeInTheDocument()
  })
})

describe('nav rail trigger', () => {
  it('uses a different icon and label for each state instead of a toggle', () => {
    setup()
    const open = screen.getByRole('button', { name: '收起侧边栏' })
    expect(open).not.toHaveAttribute('aria-expanded')
    const openIcon = open.innerHTML
    fireEvent.click(open)
    const closed = screen.getByRole('button', { name: '展开侧边栏' })
    expect(closed.innerHTML).not.toBe(openIcon)
  })
})
