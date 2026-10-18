import type { Folder, FolderRole } from '@/generated/accounts'

/** What the message list shows: one account folder, or a role across all accounts. */
export type Mailbox = { kind: 'unified'; role: FolderRole } | { kind: 'folder'; folder: Folder }

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
  return mailbox.kind === 'unified' ? UNIFIED_TITLES[mailbox.role] : folderLabel(mailbox.folder)
}
