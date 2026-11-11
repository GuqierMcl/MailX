import { t } from '@/i18n'
import type { EmailSummary } from '@/generated/accounts'
import { isGivenUp } from './preview-gave-up'

/** The preview line of a list or conversation row; empty until it is known. */
export function previewText(email: EmailSummary) {
  if (email.bodyState === 'ready' || email.previewState === 'ready') return email.preview
  if (email.previewState === 'missing' && !isGivenUp(email.id)) return t('mail:previewLoading')
  return ''
}
