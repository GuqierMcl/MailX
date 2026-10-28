import { useEffect, useState } from 'react'
import type { WindowChrome, WindowHost } from './window-host'

export function useWindowChrome(host: WindowHost | null) {
  const [chrome, setChrome] = useState<WindowChrome | null>(null)

  useEffect(() => {
    if (!host) return
    let active = true
    const stop = host.onStateChange((state) => setChrome((current) => current && { ...current, state }))
    host.getChrome().then(
      (next) => active && setChrome(next),
      () => undefined,
    )
    return () => {
      active = false
      stop()
    }
  }, [host])

  return chrome
}

/** Lets the stylesheet draw the canvas translucent once the host has put a material behind it. */
export function useTranslucentCanvas(chrome: WindowChrome | null) {
  const translucent = chrome?.translucent ?? false
  useEffect(() => {
    document.documentElement.toggleAttribute('data-translucent', translucent)
    return () => document.documentElement.removeAttribute('data-translucent')
  }, [translucent])
}
