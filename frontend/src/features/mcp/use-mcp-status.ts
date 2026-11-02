import { useEffect, useState } from 'react'
import type { MailxClient } from '@/lib/mailx-client'
import type { McpStatus } from './types'

/**
 * Live MCP server status. `null` until the host has answered once; events that
 * arrive before the initial read are newer than it and win.
 */
export function useMcpStatus(client: MailxClient): McpStatus | null {
  const [status, setStatus] = useState<McpStatus | null>(null)
  useEffect(() => {
    let live = true
    const stop = client.onMcpStatus((next) => { if (live) setStatus(next) })
    client.getMcpStatus().then(
      (initial) => { if (live) setStatus((current) => current ?? initial) },
      () => undefined,
    )
    return () => { live = false; stop() }
  }, [client])
  return status
}
