// Copies brand files into the site before every dev and build run. They are
// generated from branding/logo.svg (`bun run icons:generate`); the copies here
// are ignored by git so there is still one source.
import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'

const site = resolve(import.meta.dirname, '..')
const root = resolve(site, '..')

const files = [
  ['frontend/public/favicon.ico', 'public/favicon.ico'],
  ['frontend/public/favicon.svg', 'public/favicon.svg'],
  ['frontend/public/favicon-16x16.png', 'public/favicon-16x16.png'],
  ['frontend/public/favicon-32x32.png', 'public/favicon-32x32.png'],
  ['frontend/public/apple-touch-icon.png', 'public/apple-touch-icon.png'],
  ['branding/social/open-graph-light.png', 'public/og-image.png'],
  ['branding/variants/logo-horizontal-black.svg', 'src/assets/brand/logo-horizontal-black.svg'],
  ['branding/variants/logo-horizontal-white.svg', 'src/assets/brand/logo-horizontal-white.svg'],
  ['branding/variants/mark-black.svg', 'src/assets/brand/mark-black.svg'],
  ['branding/variants/mark-white.svg', 'src/assets/brand/mark-white.svg'],
]

for (const [from, to] of files) {
  const target = resolve(site, to)
  mkdirSync(dirname(target), { recursive: true })
  copyFileSync(resolve(root, from), target)
}
