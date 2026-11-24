/**
 * The guide's sidebar, by page id without its language; titles come from
 * each page. Listed by hand because Starlight's autogeneration only looks in
 * its own content directory. guides.test.ts checks that every page is listed
 * and exists in both languages.
 */
export const sidebar = [
  { label: '指南', translations: { en: 'Guide' }, items: ['docs', 'docs/privacy'] },
]
