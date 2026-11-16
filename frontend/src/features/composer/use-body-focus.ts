import { useCallback, useEffect, useRef, useState, type FocusEvent, type KeyboardEvent, type PointerEvent } from 'react'

/** A click's focus waits this long after the button is released, so a double or triple click lands on the text it aimed at. */
export const POINTER_COLLAPSE_DELAY_MS = 400

/**
 * Whether the user is working in the body, for collapsing the header
 * (sending.md section 17.2). Keyboard focus counts at once; focus from a click
 * counts once the pointer is up and no further press follows, because the
 * body moves up as the header shrinks and a drag or double click in progress
 * would otherwise select different text. Typing counts at once.
 *
 * The returned handlers go on an element wrapping the body editor; `leave`
 * is for focus reaching a collapsible header field.
 */
export function useBodyFocus() {
  const [active, setActive] = useState(false)
  const pressed = useRef(false)
  const waiting = useRef(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const area = useRef<HTMLElement | null>(null)
  const release = useRef<(() => void) | null>(null)

  const stop = useCallback(() => {
    clearTimeout(timer.current)
    waiting.current = false
    release.current?.()
    release.current = null
  }, [])
  useEffect(() => stop, [stop])

  const enter = useCallback(() => {
    stop()
    pressed.current = false
    setActive(true)
  }, [stop])

  const leave = useCallback(() => {
    stop()
    pressed.current = false
    setActive(false)
  }, [stop])

  const onPointerDownCapture = useCallback((event: PointerEvent<HTMLElement>) => {
    if (event.button !== 0) return
    stop()
    pressed.current = true
    area.current = event.currentTarget
    const up = () => {
      release.current?.()
      release.current = null
      pressed.current = false
      waiting.current = true
      timer.current = setTimeout(() => {
        waiting.current = false
        if (area.current?.contains(document.activeElement)) setActive(true)
      }, POINTER_COLLAPSE_DELAY_MS)
    }
    window.addEventListener('pointerup', up, { once: true })
    window.addEventListener('pointercancel', up, { once: true })
    release.current = () => {
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)
    }
  }, [stop])

  const onFocus = useCallback((event: FocusEvent<HTMLElement>) => {
    area.current = event.currentTarget
    if (!pressed.current && !waiting.current) enter()
  }, [enter])

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLElement>) => {
    if (event.key === 'Tab' || event.key === 'Escape') return
    if (!active) enter()
  }, [active, enter])

  return { active, leave, handlers: { onPointerDownCapture, onFocus, onKeyDown } }
}
