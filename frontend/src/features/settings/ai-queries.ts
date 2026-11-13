import { useEffect, useRef } from 'react'
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { errorText } from '@/i18n/errors'
import type { AiConnectionState, AiModelRef, AiState } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { aiKeys } from '@/lib/query-client'
import type { ModelGroup } from './ai-model-picker'

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

/** The model that translates mail: the chosen one, else the default, as long as its connection still exists. */
export function translationTarget(ai: AiState | undefined): { model: AiModelRef; connection: AiConnectionState } | null {
  const model = ai?.translationModel ?? ai?.defaultModel ?? null
  const connection = model ? ai?.connections.find((entry) => entry.id === model.connectionId) : undefined
  return model && connection ? { model, connection } : null
}

/** The AI settings, shared by the AI pane, the translation pane and the reader through one cache entry. */
export function useAiState(client: MailxClient, enabled = true) {
  return useQuery({ queryKey: aiKeys.state(), queryFn: () => client.aiState(), enabled })
}

/** Lists are fetched live each time the picker opens and are not kept. */
export function useModelGroups(client: MailxClient, ai: AiState, open: boolean): ModelGroup[] {
  const lists = useQueries({
    queries: ai.connections.map((connection) => ({
      queryKey: aiKeys.models(connection.id), queryFn: () => client.aiListModels(connection.id),
      enabled: open, staleTime: 0, gcTime: 60_000, refetchOnMount: 'always' as const,
    })),
  })
  return ai.connections.map((connection, index) => {
    const list = lists[index]
    return { id: connection.id, name: connection.name, providerId: connection.providerId, models: list.data, loading: list.isFetching && !list.data, failure: list.isError ? errorText(list.error) : null }
  })
}
