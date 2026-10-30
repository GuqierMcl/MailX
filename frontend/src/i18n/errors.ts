import { MailxClientError } from '@/lib/mailx-client'
import { t } from './index'
import messages from '../../../locales/zh-CN/errors.json'

export type ErrorContext = 'account' | { emailAddress?: string }

export function errorText(error: unknown, context?: ErrorContext): string {
  const failure: unknown = error instanceof MailxClientError ? error.failure : error
  if (failure && typeof failure === 'object' && 'code' in failure && typeof failure.code === 'string') {
    const code = failure.code as keyof typeof messages
    if (code === 'validation' && 'field' in failure && typeof failure.field === 'string') {
      const field = failure.field
      if (/\.port$|^proxyPort$/.test(field)) return t('accounts:invalidPort')
      if (/\.host$|^proxyHost$/.test(field)) return t('accounts:invalidHost')
      if (/\.username$/.test(field)) return t('accounts:loginRequired')
      if (field === 'emailAddress') return typeof context === 'object' && context.emailAddress
        ? context.emailAddress : t('accounts:invalidEmail')
      if (field === 'proxyUsername') return t('accounts:proxyUsernameRequired')
      if (/Password$|^sharedPassword$/.test(field)) return t('accounts:passwordRequired')
      if (field === 'text') return t('mail:searchTextInvalid')
      if (field === 'displayName') return t('accounts:accountNameRequired')
    }
    if (context === 'account' && code === 'notFound') return t('errors:accountNotFound')
    // Only controlled catalog entries can be shown, never raw IPC/OS error text.
    if (Object.prototype.hasOwnProperty.call(messages, code)) return t(`errors:${code}`)
  }
  return t('errors:internal')
}
