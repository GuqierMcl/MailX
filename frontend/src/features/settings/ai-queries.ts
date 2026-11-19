import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { errorText } from '@/i18n/errors'
import type { AiConnectionState, AiModel, AiModelRef, AiModelSource, AiState } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { aiKeys } from '@/lib/query-client'

/** Catalog from the local copy first; when it is out of date, a refresh swaps in quietly and a failed one is ignored. */
export function useAiCatalog(client: MailxClient, enabled = true) {
  const queryClient = useQueryClient()
  const catalog = useQuery({ queryKey: aiKeys.catalog(), queryFn: () => client.aiCatalog(false), enabled })
  const refresh = useQuery({
    queryKey: aiKeys.catalogRefresh(),
    queryFn: async () => {
      const fresh = await client.aiCatalog(true)
      queryClient.setQueryData(aiKeys.catalog(), fresh)
      return fresh
    },
    enabled: enabled && catalog.data?.stale === true,
    staleTime: 60_000,
  })
  return { data: catalog.data, isPending: catalog.isPending, refreshing: refresh.isFetching }
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

export type ModelPickerConnection = Pick<AiConnectionState, 'id' | 'name' | 'providerId'> & {
  // The first-connection wizard can reuse the result of its connection check.
  initialModels?: AiModel[] | null
}

/** Live lists are scoped to an open picker. Presets follow the cached catalog's revision. */
export function useModelGroups(client: MailxClient, connections: ModelPickerConnection[], source: AiModelSource) {
  const presets = source === 'catalog'
  const catalog = useAiCatalog(client, presets)
  const lists = useQueries({
    queries: connections.map((connection) => ({
      queryKey: aiKeys.models(connection.id, source, presets ? catalog.data?.updatedAt ?? null : null),
      queryFn: () => client.aiListModels(connection.id, source),
      enabled: !presets || !catalog.isPending,
      initialData: presets ? undefined : connection.initialModels,
      staleTime: presets || connection.initialModels !== undefined ? Infinity : 0,
      gcTime: 0,
    })),
  })
  return connections.map((connection, index) => {
    const list = lists[index]
    return {
      connection, models: list.data,
      loading: list.isPending || (presets && catalog.refreshing && list.data?.length === 0),
      failure: list.isError ? errorText(list.error) : null,
    }
  })
}
