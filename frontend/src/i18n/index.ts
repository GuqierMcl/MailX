import i18next from 'i18next'
import { initReactI18next } from 'react-i18next'

const resources: Record<string, Record<string, Record<string, unknown>>> = {}
const catalogs = import.meta.glob('../../../locales/*/*.json', { eager: true, import: 'default' })
for (const [path, catalog] of Object.entries(catalogs)) {
  const match = /\/([^/]+)\/([^/]+)\.json$/.exec(path)
  if (match) (resources[match[1]] ??= {})[match[2]] = catalog as Record<string, unknown>
}

export const i18n = i18next.createInstance()
void i18n.use(initReactI18next).init({
  resources,
  lng: 'zh-CN',
  supportedLngs: ['zh-CN', 'en'],
  fallbackLng: 'zh-CN',
  defaultNS: 'common',
  initAsync: false,
  interpolation: { escapeValue: false },
  react: { useSuspense: false },
})

// Resolve at invocation time, including calls outside React event handlers.
export const t: typeof i18n.t = i18n.t.bind(i18n)
