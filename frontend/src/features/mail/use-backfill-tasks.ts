import { useEffect, useState } from 'react'
import type { AccountBackfillTask } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'

/** Tracks historical backfill tasks across initial load and live host events. */
export function useBackfillTasks(client: MailxClient) {
  const [tasks, setTasks] = useState<AccountBackfillTask[]>([])
  useEffect(() => {
    let active = true
    const unlisten = client.onBackfill((next) => {
      if (active) setTasks(next)
    })
    void client.listBackfillTasks().then((initial) => {
      if (active) setTasks(initial)
    }, () => undefined)
    return () => {
      active = false
      unlisten()
    }
  }, [client])
  return { tasks, setTasks }
}
