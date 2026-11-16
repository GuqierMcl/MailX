import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'

/** Closer than this to the end counts as "at the bottom". */
const NEAR_PX = 48

function jump(node: HTMLElement, behavior: ScrollBehavior) {
  if (typeof node.scrollTo === 'function') node.scrollTo({ top: node.scrollHeight, behavior })
  else node.scrollTop = node.scrollHeight
}

/**
 * Keeps a scroll area at its end while the content grows, unless the user scrolled up (ai-chat-ui.md
 * 6.2). `version` is anything that changes when there is more to see (the messages). Following is
 * given up when the user scrolls away from the end and taken up again when they come back, or
 * with `scrollToBottom`. That call scrolls smoothly (unless `reduced`); following new text jumps at
 * once, so it never lags behind a streaming reply. The area opens at its end.
 */
export function useStickToBottom(version: unknown, { reduced = false, enabled = true }: { reduced?: boolean; enabled?: boolean } = {}) {
  const element = useRef<HTMLDivElement | null>(null)
  const following = useRef(true)
  const lastTop = useRef(0)
  const [atBottom, setAtBottom] = useState(true)

  const distance = () => {
    const node = element.current
    return node ? node.scrollHeight - node.scrollTop - node.clientHeight : 0
  }
  const scrollToBottom = useCallback((smooth = true) => {
    const node = element.current
    if (!node) return
    following.current = true
    setAtBottom(true)
    jump(node, smooth && !reduced ? 'smooth' : 'auto')
  }, [reduced])

  const onScroll = useCallback(() => {
    const node = element.current
    if (!node) return
    const near = distance() <= NEAR_PX
    // Moving up and away is the user taking over; our own smooth scroll only ever moves down.
    if (node.scrollTop < lastTop.current - 1 && !near) following.current = false
    if (near) following.current = true
    lastTop.current = node.scrollTop
    setAtBottom(near)
  }, [])

  // Wheel and touch upward are intent too, before any scroll event moves the position.
  const onWheel = useCallback((event: { deltaY: number }) => {
    if (event.deltaY < 0 && distance() > 0) following.current = false
  }, [])

  useLayoutEffect(() => {
    if (enabled && element.current) jump(element.current, 'auto')
  }, [enabled])

  const previous = useRef(version)
  useLayoutEffect(() => {
    if (!enabled || !following.current || previous.current === version) { previous.current = version; return }
    previous.current = version
    const node = element.current
    if (node) jump(node, 'auto')
  }, [version, enabled])

  // Growth that is not a render of ours (images, a card folding) still keeps the end in view.
  useEffect(() => {
    const node = element.current
    if (!node || typeof ResizeObserver === 'undefined' || !enabled) return
    const content = node.firstElementChild
    if (!content) return
    const observer = new ResizeObserver(() => {
      if (following.current) jump(node, 'auto')
      else setAtBottom(distance() <= NEAR_PX)
    })
    observer.observe(content)
    return () => observer.disconnect()
  }, [enabled])

  const attach = useCallback((node: HTMLDivElement | null) => { element.current = node }, [])
  const node = useCallback(() => element.current, [])
  return { attach, node, atBottom, scrollToBottom, onScroll, onWheel }
}
