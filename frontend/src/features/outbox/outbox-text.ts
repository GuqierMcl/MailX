import { t } from '@/i18n'
import type { OutboxError, OutboxItem, OutboxState } from '@/generated/accounts'

export const STATE_LABELS: Record<OutboxState, string> = {
  get queued() { return t('composer:detail.waiting') },
  get sending() { return t('composer:state.sending') },
  get accepted() { return t('composer:toast.sent') },
  get failed() { return t('composer:state.failed') },
  get unknown() { return t('composer:state.unknown') },
  get canceled() { return t('composer:state.canceled') },
}

/** Display reasons for the core's fixed failure codes (sending.md sections 4 and 5). */
const REASONS: Record<string, string> = {
  get connect() { return t('composer:reason.connect') },
  get proxy() { return t('composer:reason.proxy') },
  get timeout() { return t('composer:reason.timeout') },
  get network() { return t('composer:reason.network') },
  get protocol() { return t('composer:reason.protocol') },
  get server() { return t('composer:reason.server') },
  get tlsVerification() { return t('composer:reason.tlsVerification') },
  get tlsUnavailable() { return t('composer:reason.tlsUnavailable') },
  get authUnavailable() { return t('composer:reason.authUnavailable') },
  get authRejected() { return t('composer:reason.authRejected') },
  get messageTooLarge() { return t('composer:reason.messageTooLarge') },
  get credentialUntrusted() { return t('composer:reason.credentialUntrusted') },
  get credentialMissing() { return t('composer:reason.credentialMissing') },
  get secretStore() { return t('composer:reason.secretStore') },
  get account() { return t('composer:reason.account') },
  get dataMarkFailed() { return t('composer:reason.dataMarkFailed') },
  get invalidMessage() { return t('composer:reason.invalidMessage') },
  get recipientsRejected() { return t('composer:reason.recipientsRejected') },
  get rejectedAfterData() { return t('composer:reason.rejectedAfterData') },
  get retryExhausted() { return t('composer:reason.retryExhausted') },
  get accountChanged() { return t('composer:reason.accountChanged') },
  get unknown() { return t('composer:reason.unknown') },
}

export function reasonText(reason: string | null | undefined): string {
  if (!reason) return t('composer:reason.unspecified')
  return Object.hasOwn(REASONS, reason) ? REASONS[reason] : t('composer:state.failed')
}

export function errorSummary(error: OutboxError | null): string {
  return reasonText(error?.reason)
}

/** "a@x.com、b@y.com 等 5 位" */
export function recipientsPreview(item: Pick<OutboxItem, 'recipients' | 'recipientCount'>): string {
  const names = item.recipients.join('、')
  return item.recipientCount > item.recipients.length ? t('composer:recipient.preview', { names, count: item.recipientCount }) : names
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
