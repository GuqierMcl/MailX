import { createContext, useContext } from 'react'
import type { Account, Folder } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'

/** What the chat's parts need from the place that hosts them (the Now page today, the reader and composer later). */
export type ChatEnv = {
  client: MailxClient
  /** Opens a mail in the reader, as the host does for its own links. */
  onOpenEmail: (email: { id: string; accountId: string }) => void
  /** Opens a draft in the compose window. */
  onOpenDraft: (draftId: string) => void
  /** The accounts and folders as the host has them loaded: folder cards in replies are drawn from these. */
  accounts: Account[]
  folders: Folder[]
  /** Shows a folder's mail, as a click on it in the sidebar does. */
  onOpenFolder: (folder: Folder) => void
  /** Answers the pending confirmation. */
  respond: (approved: boolean) => void
  /** Asks again after a failure. */
  retry: () => void
  /** Gives the focus back to the input. */
  focusInput: () => void
  /** Calls whose undo was used. */
  undone: ReadonlySet<string>
  markUndone: (callId: string) => void
}

export const ChatEnvContext = createContext<ChatEnv | null>(null)

export function useChatEnv(): ChatEnv {
  const env = useContext(ChatEnvContext)
  if (!env) throw new Error('Chat parts need a ChatEnvContext.Provider')
  return env
}
