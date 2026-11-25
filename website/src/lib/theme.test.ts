import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

test('both dark blocks in tokens.css hold the same values', () => {
  const css = readFileSync(resolve(import.meta.dir, '../styles/tokens.css'), 'utf8')
  const values = (selector: string) => {
    const start = css.indexOf(selector)
    expect(start).toBeGreaterThan(-1)
    const block = css.slice(css.indexOf('{', start) + 1, css.indexOf('}', start))
    return block.split('\n').map((line) => line.trim()).filter((line) => line.startsWith('--'))
  }
  const chosen = values(':root[data-theme="dark"]')
  expect(chosen.length).toBeGreaterThan(10)
  expect(values(':root:not([data-theme])')).toEqual(chosen)
})
