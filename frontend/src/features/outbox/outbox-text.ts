import type { OutboxError, OutboxItem, OutboxState } from '@/generated/accounts'

export const STATE_LABELS: Record<OutboxState, string> = {
  queued: '等待发送',
  sending: '正在发送',
  accepted: '已发送',
  failed: '发送失败',
  unknown: '需要确认',
  canceled: '已撤销',
}

/** Chinese reasons for the core's fixed failure codes (sending.md sections 4 and 5). */
const REASONS: Record<string, string> = {
  connect: '无法连接发信服务器',
  proxy: '代理暂时不可用',
  timeout: '连接发信服务器超时',
  network: '网络中断',
  protocol: '发信服务器返回了无法识别的回复',
  server: '发信服务器拒绝了这封邮件',
  tlsVerification: '发信服务器的证书验证失败',
  tlsUnavailable: '发信服务器不支持加密连接',
  authUnavailable: '发信服务器不支持 MailX 可用的登录方式',
  authRejected: '发信服务器拒绝了账号登录，请检查密码或授权码',
  messageTooLarge: '邮件超过发信服务器允许的大小',
  credentialUntrusted: '发信凭证需要重新确认，请在账号设置中处理',
  credentialMissing: '没有找到发信密码，请在账号设置中重新输入',
  secretStore: '系统凭据库暂时不可用',
  account: '账号状态异常',
  dataMarkFailed: '无法记录发送进度，邮件没有发出',
  invalidMessage: '邮件内容无法发送',
  recipientsRejected: '服务器拒绝了部分收件人，邮件没有发出',
  rejectedAfterData: '服务器在收到邮件后拒绝了它',
  retryExhausted: '多次重试超过 24 小时仍未发出',
  accountChanged: '发件账号的地址已更改，这封邮件不会自动发出',
  unknown: '发送过程中连接中断，服务器可能已收到这封邮件',
}

export function reasonText(reason: string | null | undefined): string {
  if (!reason) return '原因不明'
  return REASONS[reason] ?? '发送失败'
}

export function errorSummary(error: OutboxError | null): string {
  return reasonText(error?.reason)
}

/** "a@x.com、b@y.com 等 5 位" */
export function recipientsPreview(item: Pick<OutboxItem, 'recipients' | 'recipientCount'>): string {
  const names = item.recipients.join('、')
  return item.recipientCount > item.recipients.length ? `${names} 等 ${item.recipientCount} 位` : names
}

/**
 * A failed item the user already re-sent (a newer attempt for the same draft exists)
 * has served its purpose; it would only be noise in the outbox.
 */
export function visibleOutbox(items: OutboxItem[]): OutboxItem[] {
  return items.filter((item) => {
    if (item.state !== 'failed' || !item.draftId) return true
    return !items.some((other) => other.id !== item.id && other.draftId === item.draftId && other.state !== 'failed' && other.createdAt >= item.createdAt)
  })
}
