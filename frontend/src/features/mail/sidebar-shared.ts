import type { IconSvgElement } from '@hugeicons/react'
import { Delete02Icon, Folder01Icon, InboxIcon, SentIcon, SpamIcon } from '@hugeicons/core-free-icons'
import type { Account, AccountSyncState, Folder, FolderRole } from '@/generated/accounts'
import type { Mailbox } from './mailbox'

/** Shown in the tooltip; the shortcut itself is handled by the workbench. */
export const COMPOSE_SHORTCUT = /Mac|iPhone|iPad/.test(globalThis.navigator?.platform ?? '') ? '⌘N' : 'Ctrl N'

/**
 * Icons of the folders under an account. Server drafts and archive folders look
 * like any other folder there: the top's 草稿 and 归档 are MailX's local views.
 */
export const ROLE_ICONS: Record<FolderRole, IconSvgElement> = {
  inbox: InboxIcon,
  drafts: Folder01Icon,
  sent: SentIcon,
  archive: Folder01Icon,
  junk: SpamIcon,
  trash: Delete02Icon,
  other: Folder01Icon,
}

export type MailSidebarProps = {
  accounts: Account[]
  folders: Folder[]
  loading: boolean
  /** Latest sync state by account id; missing means no sync has reported yet. */
  syncStates: Record<string, AccountSyncState>
  mailbox: Mailbox
  onSelect: (mailbox: Mailbox) => void
  onOpenAccounts: () => void
  onOpenSettings: () => void
  onCompose: () => void
  /** Unfinished drafts; the entry is always there. */
  draftCount: number
  /** Unfinished sends (queued, sending, failed, unknown); the entry only appears when there are some. */
  outboxCount: number
  /** Starts the account's next sync round now. */
  onRefresh: (accountId: string) => Promise<void>
}
