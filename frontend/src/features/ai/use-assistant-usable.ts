import { assistantTarget } from '@/features/ai/assistant-target'
import { useAiState } from '@/features/settings/ai-queries'
import { useSettings } from '@/features/settings/settings-queries'
import type { MailxClient } from '@/lib/mailx-client'

/**
 * Whether the assistant can be used now: it is on in the settings and the model it would use still has
 * its connection (`assistantTarget`). Nothing is asked of the host until the assistant is on. `service`
 * names the connection, for the consent dialog.
 */
export function useAssistantUsable(client: MailxClient): { usable: boolean; service: string } {
  const settings = useSettings(client)
  const assistant = settings.data?.settings.assistant
  const on = assistant?.enabled === true
  const ai = useAiState(client, on)
  const target = on ? assistantTarget(ai.data, assistant) : null
  return { usable: target !== null, service: target?.connection.name ?? '' }
}
