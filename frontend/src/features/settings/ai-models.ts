import type { AiModel, AiModelRef } from '@/generated/accounts'

/** 200000 → "200K", 1048576 → "1M". */
export function formatTokens(count: number) {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
  return `${Math.max(1, Math.round(count / 1000))}K`
}

/** What is saved for a chosen model: the catalog's facts when known, nothing for a typed id. */
export function modelRef(connectionId: string, model: AiModel | string): AiModelRef {
  return typeof model === 'string'
    ? { connectionId, modelId: model, name: model, contextLength: null, outputLength: null }
    : { connectionId, modelId: model.id, name: model.name, contextLength: model.contextLength, outputLength: model.outputLength }
}
