import { useEffect, useRef } from 'react'
import { matchShortcut, type SingleKeyShortcutId } from './shortcuts'

/**
 * Runs the main window's single-key shortcuts. `run` returns whether the
 * shortcut did something; only then is the key press consumed, so a key with
 * nothing to act on keeps its usual meaning (an arrow still scrolls).
 */
export function useMailShortcuts(enabled: boolean, run: (id: SingleKeyShortcutId) => boolean) {
  const latest = useRef(run)
  useEffect(() => {
    latest.current = run
  })
  useEffect(() => {
    if (!enabled) return
    const onKeyDown = (event: KeyboardEvent) => {
      const id = matchShortcut(event)
      if (id && latest.current(id)) event.preventDefault()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [enabled])
}
