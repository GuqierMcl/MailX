/**
 * The guide's sidebar, by page id without its language; titles come from
 * each page. Listed by hand because Starlight's autogeneration only looks in
 * its own content directory. guides.test.ts checks that every page is listed
 * and exists in both languages.
 */
export const sidebar = [
  {
    label: '开始使用',
    translations: { en: 'Getting Started' },
    items: ['docs', 'docs/install', 'docs/add-account'],
  },
  {
    label: '日常使用',
    translations: { en: 'Everyday Mail' },
    items: ['docs/reading', 'docs/writing'],
  },
  {
    label: '翻译与 AI',
    translations: { en: 'Translation & AI' },
    items: ['docs/translation', 'docs/ai', 'docs/external-ai'],
  },
  {
    label: '设置、隐私与帮助',
    translations: { en: 'Settings, Privacy & Help' },
    items: ['docs/settings', 'docs/privacy', 'docs/troubleshooting'],
  },
]
