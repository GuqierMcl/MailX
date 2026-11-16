import type { MouseEvent as ReactMouseEvent } from 'react'
import type { BaseUIEvent } from '@base-ui/react/types'

/** Menus re-sent at the row by `anchorContextMenuAt`; they are never moved again. */
const reanchored = new WeakSet<Event>()

/**
 * Shift+F10 and the menu key send `contextmenu` with a point that may lie
 * outside the row, often (0, 0), and the menu opens at that point. Such an
 * event is replaced by one at the row: 16px in from its left edge, vertically
 * centred. A mouse right-click always lands inside the row and is left alone.
 * Returns true when the event was replaced.
 */
export function anchorContextMenuAt(event: BaseUIEvent<ReactMouseEvent<HTMLElement>>, row: HTMLElement) {
  if (reanchored.has(event.nativeEvent)) return false
  const rect = row.getBoundingClientRect()
  const { clientX: x, clientY: y } = event
  if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) return false
  const replay = new MouseEvent('contextmenu', {
    bubbles: true,
    cancelable: true,
    composed: true,
    button: 2,
    clientX: rect.left + 16,
    clientY: rect.top + rect.height / 2,
  })
  reanchored.add(replay)
  event.preventBaseUIHandler()
  event.preventDefault()
  event.target.dispatchEvent(replay)
  return true
}
