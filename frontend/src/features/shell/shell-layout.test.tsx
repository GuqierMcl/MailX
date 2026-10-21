import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ShellLayout } from './shell-layout'
import { NAV_DEFAULT, NAV_MAX, NAV_MIN } from './nav-width'

const setup = (navWidth = NAV_DEFAULT) => {
  const onNavWidthChange = vi.fn()
  render(
    <ShellLayout navLabel="导航" nav={<p>菜单</p>} navWidth={navWidth} onNavWidthChange={onNavWidthChange}>
      <p>内容</p>
    </ShellLayout>,
  )
  return { handle: screen.getByRole('separator', { name: '调整侧边栏宽度' }), onNavWidthChange }
}
const width = () => Number(screen.getByRole('separator').getAttribute('aria-valuenow'))

describe('nav rail resizing', () => {
  it('shows the saved width', () => {
    setup(300)
    expect(width()).toBe(300)
  })

  it('saves a drag once, clamped, when it ends', () => {
    const { handle, onNavWidthChange } = setup()
    handle.setPointerCapture = () => undefined
    fireEvent.pointerDown(handle, { button: 0, clientX: 100, pointerId: 1 })
    fireEvent.pointerMove(handle, { clientX: 5000, pointerId: 1 })
    expect(width()).toBe(NAV_MAX)
    expect(onNavWidthChange).not.toHaveBeenCalled()
    fireEvent.pointerUp(handle, { pointerId: 1 })
    expect(onNavWidthChange).toHaveBeenCalledExactlyOnceWith(NAV_MAX)
  })

  it('adjusts with the keyboard and saves on key release', () => {
    const { handle, onNavWidthChange } = setup()
    fireEvent.keyDown(handle, { key: 'ArrowRight' })
    expect(width()).toBe(NAV_DEFAULT + 8)
    fireEvent.keyUp(handle, { key: 'ArrowRight' })
    expect(onNavWidthChange).toHaveBeenCalledExactlyOnceWith(NAV_DEFAULT + 8)
    fireEvent.keyDown(handle, { key: 'Home' })
    fireEvent.keyDown(handle, { key: 'ArrowLeft', shiftKey: true })
    expect(width()).toBe(NAV_MIN)
  })

  it('resets to the default on double click', () => {
    const { handle, onNavWidthChange } = setup(300)
    fireEvent.doubleClick(handle)
    expect(width()).toBe(NAV_DEFAULT)
    expect(onNavWidthChange).toHaveBeenCalledWith(NAV_DEFAULT)
  })
})
