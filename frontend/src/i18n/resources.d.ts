import 'i18next'
import common from '../../../locales/zh-CN/common.json'
import settings from '../../../locales/zh-CN/settings.json'
import accounts from '../../../locales/zh-CN/accounts.json'
import mail from '../../../locales/zh-CN/mail.json'
import composer from '../../../locales/zh-CN/composer.json'
import errors from '../../../locales/zh-CN/errors.json'

declare module 'i18next' {
  interface CustomTypeOptions {
    defaultNS: 'common'
    returnNull: false
    resources: { common: typeof common; settings: typeof settings; accounts: typeof accounts; mail: typeof mail; composer: typeof composer; errors: typeof errors }
  }
}
