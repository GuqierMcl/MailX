import { createContext, useContext } from 'react'
import type { AssistantChat } from './use-assistant-chat'

export const AssistantChatContext = createContext<AssistantChat | null>(null)

/** The shared conversation, or null where no provider is mounted. */
export const useSharedAssistantChat = () => useContext(AssistantChatContext)
