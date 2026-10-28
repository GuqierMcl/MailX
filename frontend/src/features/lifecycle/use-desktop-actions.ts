import { useEffect, useRef } from 'react'
import type { DesktopAction } from './lifecycle-client'
import { useLifecycle } from './lifecycle-context'

/** The tray only enables actions once their workbench listener is installed. */
export function useDesktopActions(handle: (action: DesktopAction) => void) {
  const lifecycle = useLifecycle()
  const client = lifecycle?.client
  const latest = useRef(handle)
  useEffect(() => { latest.current = handle }, [handle])
  useEffect(() => {
    if (!client) return
    let disposed = false
    let stop: (() => void) | undefined
    let ready = false
    void client.onDesktopAction((action) => { if (!disposed) latest.current(action) }).then(async (unsubscribe) => {
      if (disposed) { unsubscribe(); return }
      stop = unsubscribe
      ready = true
      await client.setDesktopReady(true)
    }).catch(() => undefined)
    return () => {
      disposed = true
      stop?.()
      if (ready) void client.setDesktopReady(false).catch(() => undefined)
    }
  }, [client])
}
