import { useEffect, useRef } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { MailxClient } from '@/lib/mailx-client'
import { aiKeys } from '@/lib/query-client'

/** Catalog from the local copy first; when it is out of date, a refresh swaps in quietly and a failed one is ignored. */
export function useAiCatalog(client: MailxClient, enabled = true) {
  const queryClient = useQueryClient()
  const catalog = useQuery({ queryKey: aiKeys.catalog(), queryFn: () => client.aiCatalog(false), enabled })
  const refreshed = useRef(false)
  const stale = catalog.data?.stale === true
  useEffect(() => {
    if (!enabled || !stale || refreshed.current) return
    refreshed.current = true
    client.aiCatalog(true).then((fresh) => queryClient.setQueryData(aiKeys.catalog(), fresh), () => undefined)
  }, [client, enabled, queryClient, stale])
  return catalog
}
