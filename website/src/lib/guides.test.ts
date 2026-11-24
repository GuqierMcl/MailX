import { expect, test } from 'bun:test'
import { Glob } from 'bun'
import { resolve } from 'node:path'
import { guideId } from './guides'
import { sidebar } from './sidebar.mjs'

test('guideId maps each locale to its URL', () => {
  expect(guideId('zh-CN/index.md')).toBe('zh-CN/docs')
  expect(guideId('zh-CN/privacy.md')).toBe('zh-CN/docs/privacy')
  expect(guideId('en/index.md')).toBe('en/docs')
  expect(guideId('en/accounts/Gmail.mdx')).toBe('en/docs/accounts/gmail')
  expect(guideId('en/accounts/index.md')).toBe('en/docs/accounts')
})

test('guideId refuses files outside the two locales', () => {
  expect(() => guideId('README.md')).toThrow()
  expect(() => guideId('fr/index.md')).toThrow()
})

test('every guide page exists in both languages and is in the sidebar', () => {
  const root = resolve(import.meta.dir, '../../../docs/guides')
  const pages = (locale: string) => [...new Glob('**/*.{md,mdx}').scanSync(resolve(root, locale))]
    .filter((file) => !file.endsWith('README.md'))
    .map((file) => guideId(`${locale}/${file}`).slice(locale.length + 1))
    .sort()
  const listed = sidebar.flatMap((group) => group.items).sort()
  expect(pages('zh-CN')).toEqual(listed)
  expect(pages('en')).toEqual(listed)
})
