import type { EmailListQuery, MailAddress } from '@/generated/accounts'

export function emailQuery(query: Partial<EmailListQuery>): EmailListQuery {
  return {
    accountIds: null,
    folderId: null,
    folderRole: null,
    text: null,
    unread: null,
    hasAttachment: null,
    receivedAfter: null,
    receivedBefore: null,
    limit: null,
    cursor: null,
    ...query,
  }
}

export function displayName(address: MailAddress) {
  return address.name?.trim() || address.address
}

export function initials(address: MailAddress) {
  return Array.from(displayName(address))[0]?.toUpperCase() ?? '?'
}

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()

/** List timestamps: time today, "昨天", weekday this week, otherwise a date. */
export function formatListTime(value: string, now = new Date()) {
  const date = new Date(value)
  if (sameDay(date, now)) return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  const yesterday = new Date(now)
  yesterday.setDate(now.getDate() - 1)
  if (sameDay(date, yesterday)) return '昨天'
  const days = (now.getTime() - date.getTime()) / 86_400_000
  if (days < 7) return date.toLocaleDateString('zh-CN', { weekday: 'short' })
  if (date.getFullYear() === now.getFullYear()) return date.toLocaleDateString('zh-CN', { month: 'numeric', day: 'numeric' })
  return date.toLocaleDateString('zh-CN', { year: 'numeric', month: 'numeric', day: 'numeric' })
}

export function formatFullTime(value: string) {
  return new Date(value).toLocaleString('zh-CN', {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
    weekday: 'short',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}
