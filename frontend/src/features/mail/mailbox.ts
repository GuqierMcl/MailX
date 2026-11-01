import { t } from '@/i18n'
import type { Folder, FolderRole } from '@/generated/accounts'

/**
 * What the island shows: one account folder, a role across all accounts, the two
 * local views that hold unfinished mail (sending.md section 7), or the locally
 * archived inbox mail of one account (or of all). `archived` is the local archive,
 * not the server's `archive` folder, which is an ordinary `folder`.
 */
export type Mailbox =
  | { kind: 'unified'; role: FolderRole }
  | { kind: 'folder'; folder: Folder }
  | { kind: 'archived'; accountId: string | null }
  | { kind: 'drafts' }
  | { kind: 'outbox' }

export const UNIFIED_TITLES: Record<FolderRole, string> = {
  get inbox() { return t('mail:allInboxes') },
  get drafts() { return t('mail:allDrafts') },
  get sent() { return t('mail:allSent') },
  get archive() { return t('mail:allArchive') },
  get junk() { return t('mail:allJunk') },
  get trash() { return t('mail:allTrash') },
  get other() { return t('mail:allMail') },
}

const ROLE_LABELS: Record<FolderRole, string | null> = {
  get inbox() { return t('mail:inbox') },
  get drafts() { return t('mail:drafts') },
  get sent() { return t('mail:sent') },
  get archive() { return t('mail:archive') },
  get junk() { return t('mail:junk') },
  get trash() { return t('mail:trash') },
  other: null,
}

/** System folders get the same label on every server ("Sent Messages", "已发送邮件" → 已发送). */
export function folderLabel(folder: Folder) {
  return ROLE_LABELS[folder.role] ?? folder.name
}

export function mailboxTitle(mailbox: Mailbox) {
  switch (mailbox.kind) {
    case 'unified': return UNIFIED_TITLES[mailbox.role]
    case 'folder': return folderLabel(mailbox.folder)
    case 'archived': return t('mail:allArchived')
    case 'drafts': return t('mail:drafts')
    case 'outbox': return t('mail:outbox')
  }
}

/** Whether the island shows mail (list and reader) rather than a local view. */
export const isMailView = (mailbox: Mailbox) => mailbox.kind === 'unified' || mailbox.kind === 'folder' || mailbox.kind === 'archived'
