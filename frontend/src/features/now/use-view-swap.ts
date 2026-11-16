import { useCallback } from 'react'
import { flushSync } from 'react-dom'

type TransitionDocument = Document & { startViewTransition?: (update: () => void) => { finished: Promise<unknown> } }

/**
 * Runs a view change inside a View Transition when the browser has them (ai-chat-ui.md 6.2). The
 * `direction` is written on the root for the length of the transition; index.css names the page body
 * and the input only then, so nothing is captured at other times. Returns whether a transition ran:
 * when it did not, the caller's own CSS cross-fade applies (and with reduced motion, nothing does).
 */
export function useViewSwap(reduced: boolean) {
  return useCallback((direction: 'conversation' | 'overview', update: () => void): boolean => {
    const doc = document as TransitionDocument
    if (reduced || typeof doc.startViewTransition !== 'function') {
      update()
      return false
    }
    const root = document.documentElement
    root.dataset.nowTransition = direction
    let transition: { finished: Promise<unknown> }
    try {
      transition = doc.startViewTransition(() => { flushSync(update) })
    } catch {
      delete root.dataset.nowTransition
      update()
      return false
    }
    const done = () => { delete root.dataset.nowTransition }
    transition.finished.then(done, done)
    return true
  }, [reduced])
}
