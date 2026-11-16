import type { ReactNode } from 'react'
import type { MailxClient } from '@/lib/mailx-client'
import { AssistantChatContext } from './assistant-chat-context'
import { useAssistantChat } from './use-assistant-chat'

/**
 * Owns the assistant conversation for as long as it is mounted, so it outlives the pages that show it
 * (the Now page closes when another mailbox opens). Put it high, above the views; its updates reach
 * only the components that read the chat, not the children it wraps.
 */
export function AssistantChatProvider({ client, children }: { client: MailxClient; children: ReactNode }) {
  const chat = useAssistantChat(client)
  return <AssistantChatContext value={chat}>{children}</AssistantChatContext>
}
