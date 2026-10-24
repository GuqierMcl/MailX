import type { Folder, FolderRole } from '@/generated/accounts'

/**
 * What the island shows: one account folder, a role across all accounts, or the two
 * local views that hold unfinished mail (sending.md section 7).
 */
export type Mailbox =
  | { kind: 'unified'; role: FolderRole }
  | { kind: 'folder'; folder: Folder }
  | { kind: 'drafts' }
  | { kind: 'outbox' }

export const UNIFIED_TITLES: Record<FolderRole, string> = {
  inbox: '全部收件箱',
  drafts: '全部草稿',
  sent: '全部已发送',
  archive: '全部归档',
  junk: '全部垃圾邮件',
  trash: '全部已删除',
  other: '全部邮件',
}

const ROLE_LABELS: Record<FolderRole, string | null> = {
  inbox: '收件箱',
  drafts: '草稿',
  sent: '已发送',
  archive: '归档',
  junk: '垃圾邮件',
  trash: '已删除',
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
    case 'drafts': return '草稿'
    case 'outbox': return '发件箱'
  }
}

/** Whether the island shows mail (list and reader) rather than a local view. */
export const isMailView = (mailbox: Mailbox) => mailbox.kind === 'unified' || mailbox.kind === 'folder'
