import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { parseChangelog, parseEntry, renderInline, sectionLabel, splitEntry } from './changelog'

describe('splitEntry', () => {
  test('splits a plain entry', () => {
    expect(splitEntry('Show the version. / 显示版本号。')).toEqual({ en: 'Show the version.', 'zh-CN': '显示版本号。' })
  })

  test('keeps slashes that belong to either half', () => {
    expect(splitEntry('163 / 126 / yeah.net use their own provider icons. / 163 / 126 / yeah.net 使用各自的服务商图标。')).toEqual({
      en: '163 / 126 / yeah.net use their own provider icons.',
      'zh-CN': '163 / 126 / yeah.net 使用各自的服务商图标。',
    })
  })

  test('accepts a Chinese half that starts with Latin text', () => {
    expect(splitEntry('MCP tools were renamed. / MCP 工具已改名。')['zh-CN']).toBe('MCP 工具已改名。')
  })

  test('ignores CJK inside code spans of the English half', () => {
    expect(splitEntry('Names look like `MailX_<版本>_macos.dmg`. / 名称形如 `MailX_<版本>_macos.dmg`。').en).toBe('Names look like `MailX_<版本>_macos.dmg`.')
  })

  test('leaves an entry without a Chinese half whole', () => {
    expect(splitEntry('English only.')).toEqual({ en: 'English only.', 'zh-CN': 'English only.' })
  })
})

test('parseEntry keeps the trailing pull request link apart', () => {
  expect(parseEntry('- Fix it. / 修复。 ([#25](https://github.com/GuqierMcl/MailX/pull/25))')).toEqual({
    en: 'Fix it.',
    'zh-CN': '修复。',
    link: { label: '#25', url: 'https://github.com/GuqierMcl/MailX/pull/25' },
  })
})

test('parseChangelog skips Unreleased and empty sections', () => {
  const releases = parseChangelog([
    '- An intro line that is not an entry.',
    '## [Unreleased]',
    '### 🚀 Feature',
    '- Not yet. / 还没有。',
    '## [0.2.0] - 2026-11-12',
    '### 🚀 Feature',
    '- New. / 新的。',
    '### 🩹 Fixed',
    '',
    '## [0.1.0] - 2026-11-04',
    '### 🩹 Fixed',
    '- Old. / 旧的。',
    '[0.2.0]: https://example.com',
  ].join('\n'))
  expect(releases.map((release) => release.version)).toEqual(['0.2.0', '0.1.0'])
  expect(releases[0].date).toBe('2026-11-12')
  expect(releases[0].sections.map((section) => section.kind)).toEqual(['🚀 Feature'])
})

test('every entry in the real changelog has both halves', () => {
  const markdown = readFileSync(resolve(import.meta.dir, '../../../CHANGELOG.md'), 'utf8')
  const entries = parseChangelog(markdown).flatMap((release) => release.sections.flatMap((section) => section.entries))
  expect(entries.length).toBeGreaterThan(50)
  for (const entry of entries) {
    expect(/[㐀-鿿]/.test(entry.en.replace(/`[^`]*`/g, ''))).toBe(false)
    expect(/[㐀-鿿]/.test(entry['zh-CN'])).toBe(true)
  }
})

test('sectionLabel keeps the emoji, translates the word and keeps unknown headings', () => {
  expect(sectionLabel('🚀 Feature', 'zh-CN')).toEqual({ emoji: '🚀', label: '新功能' })
  expect(sectionLabel('🩹 Fixed', 'en')).toEqual({ emoji: '🩹', label: 'Fixed' })
  expect(sectionLabel('Something else', 'en')).toEqual({ emoji: '', label: 'Something else' })
})

test('renderInline escapes HTML and only links http(s)', () => {
  expect(renderInline('Use `<b>` and [docs](https://example.com) <i>')).toBe(
    'Use <code>&#60;b&#62;</code> and <a href="https://example.com">docs</a> &#60;i&#62;',
  )
  expect(renderInline('[x](javascript:alert(1))')).toBe('[x](javascript:alert(1))')
})
