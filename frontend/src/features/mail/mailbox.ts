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

export function mailboxTitle(mailbox: Mailbox) {
  return mailbox.kind === 'unified' ? UNIFIED_TITLES[mailbox.role] : mailbox.folder.name
}
