import type { AiConnectionState, AiModelRef, AiState, AssistantSettings } from '@/generated/accounts'

/** The model the assistant would use now: its own choice, else the default, as long as the connection still exists. */
export function assistantTarget(ai: AiState | undefined, assistant: Pick<AssistantSettings, 'model'> | undefined): { model: AiModelRef; connection: AiConnectionState } | null {
  const model = assistant?.model ?? ai?.defaultModel ?? null
  const connection = model ? ai?.connections.find((entry) => entry.id === model.connectionId) : undefined
  return model && connection ? { model, connection } : null
}
