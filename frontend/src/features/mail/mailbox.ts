import { t } from '@/i18n'
import type { Folder, FolderRole } from '@/generated/accounts'

/**
 * What the island shows: one account folder, a role across all accounts, starred
 * mail across all accounts, the two local views that hold unfinished mail
 * (sending.md section 7), or the locally archived inbox mail of one account (or
 * of all). `archived` is the local archive, not the server's `archive` folder,
 * which is an ordinary `folder`.
 */
export type Mailbox =
  | { kind: 'unified'; role: FolderRole }
  | { kind: 'folder'; folder: Folder }
  | { kind: 'starred' }
  | { kind: 'archived'; accountId: string | null }
  | { kind: 'drafts' }
  | { kind: 'outbox' }

export const UNIFIED_TITLES: Record<FolderRole, string> = {
  get inbox() { return t('mail:allInboxes') },
  get drafts() { return t('mail:allDrafts') },
  get sent() { return t('mail:allSent') },
  // Never selected: server archive folders are listed per account, and the one
  // archive the user sees across accounts is the local one.
  get archive() { return t('mail:allArchived') },
  get junk() { return t('mail:allJunk') },
  get trash() { return t('mail:allTrash') },
  get other() { return t('mail:allMail') },
}

/**
 * Server drafts and archive folders keep their own names: MailX's 草稿 and 归档
 * are local views, and the sidebar must not show two of either.
 */
const ROLE_LABELS: Record<FolderRole, string | null> = {
  get inbox() { return t('mail:inbox') },
  drafts: null,
  get sent() { return t('mail:sent') },
  archive: null,
  get junk() { return t('mail:junk') },
  get trash() { return t('mail:trash') },
  other: null,
}

/** Inbox, Sent, Junk and Trash get the same label on every server ("Sent Messages", "已发送邮件" → 已发送). */
export function folderLabel(folder: Folder) {
  return ROLE_LABELS[folder.role] ?? folder.name
}

export function mailboxTitle(mailbox: Mailbox) {
  switch (mailbox.kind) {
    case 'unified': return UNIFIED_TITLES[mailbox.role]
    case 'folder': return folderLabel(mailbox.folder)
    case 'starred': return t('mail:starredView')
    case 'archived': return mailbox.accountId ? t('mail:archivedFolder') : t('mail:allArchived')
    case 'drafts': return t('mail:drafts')
    case 'outbox': return t('mail:outbox')
  }
}

/** Whether the island shows mail (list and reader) rather than a local view. */
export const isMailView = (mailbox: Mailbox) =>
  mailbox.kind === 'unified' || mailbox.kind === 'folder' || mailbox.kind === 'starred' || mailbox.kind === 'archived'

/** Junk and trash stay out of Starred, like the MCP's unscoped searches. */
export const STARRED_EXCLUDED_ROLES: FolderRole[] = ['junk', 'trash']
