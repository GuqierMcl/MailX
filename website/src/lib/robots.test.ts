import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

// robots.txt points at the sitemap the build generates; a typo here would be
// invisible until a crawler ignores the site.
test('robots.txt allows crawlers and names the sitemap', () => {
  const text = readFileSync(resolve(import.meta.dir, '../../public/robots.txt'), 'utf8')
  expect(text).toContain('User-agent: *\nAllow: /')
  expect(text).toContain('Sitemap: https://mailxapp.com/sitemap-index.xml')
})
