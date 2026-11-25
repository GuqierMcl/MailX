/**
 * Reads CHANGELOG.md, whose entries carry English and Chinese on one line:
 * `- English sentence. / 中文句子。 ([#12](https://…))`. The site shows one
 * language at a time, so each entry is split back into its two halves.
 */

import type { Locale } from './locale.mjs'

export type Halves = Record<Locale, string>

export interface ChangelogEntry extends Halves {
  /** The trailing `([#12](url))` reference, kept for both languages. */
  link: { label: string; url: string } | null
}

export interface ChangelogSection {
  /** The heading as written, emoji included, e.g. `🚀 Feature`. */
  kind: string
  entries: ChangelogEntry[]
}

export interface ChangelogRelease {
  version: string
  date: string
  sections: ChangelogSection[]
}

const CJK = /[　-〿㐀-鿿＀-￯]/
const SEPARATOR = ' / '

/** CJK outside `code`: file names such as `MailX_<版本>_…` may sit in the English half. */
function hasCjk(text: string): boolean {
  return CJK.test(text.replace(/`[^`]*`/g, ''))
}
const TRAILING_LINK = /\s*\(\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)\)\s*$/

/**
 * Finds where the English half ends. Both halves may contain " / " themselves
 * ("163 / 126 / yeah.net use … / 163 / 126 / yeah.net 使用…"), so a split must
 * leave no CJK on the left and some on the right, and the English half usually
 * ends a sentence. Code spans do not count as CJK. Among the candidates the last sentence ending wins; without
 * one, the first candidate does.
 */
export function splitEntry(text: string): Halves {
  const candidates: number[] = []
  for (let index = text.indexOf(SEPARATOR); index >= 0; index = text.indexOf(SEPARATOR, index + 1)) {
    const left = text.slice(0, index)
    const right = text.slice(index + SEPARATOR.length)
    if (!hasCjk(left) && hasCjk(right)) candidates.push(index)
  }
  if (candidates.length === 0) return { en: text.trim(), 'zh-CN': text.trim() }
  const sentence = candidates.filter((index) => /[.!?)"'`’”]$/.test(text.slice(0, index).trimEnd()))
  const at = sentence.length > 0 ? sentence[sentence.length - 1] : candidates[0]
  return { en: text.slice(0, at).trim(), 'zh-CN': text.slice(at + SEPARATOR.length).trim() }
}

export function parseEntry(line: string): ChangelogEntry {
  let text = line.replace(/^-\s+/, '')
  let link: ChangelogEntry['link'] = null
  const match = text.match(TRAILING_LINK)
  if (match) {
    link = { label: match[1], url: match[2] }
    text = text.slice(0, match.index)
  }
  return { ...splitEntry(text), link }
}

/** Released versions only, newest first; `[Unreleased]` is left out. */
export function parseChangelog(markdown: string): ChangelogRelease[] {
  const releases: ChangelogRelease[] = []
  let release: ChangelogRelease | null = null
  let section: ChangelogSection | null = null
  for (const line of markdown.split(/\r?\n/)) {
    const heading = line.match(/^## \[([^\]]+)\](?:\s+-\s+(\S+))?/)
    if (heading) {
      section = null
      release = heading[1] === 'Unreleased' ? null : { version: heading[1], date: heading[2] ?? '', sections: [] }
      if (release) releases.push(release)
      continue
    }
    if (!release) continue
    const kind = line.match(/^### (.+)$/)
    if (kind) {
      section = { kind: kind[1].trim(), entries: [] }
      release.sections.push(section)
      continue
    }
    if (section && line.startsWith('- ')) section.entries.push(parseEntry(line))
  }
  return releases.map((item) => ({ ...item, sections: item.sections.filter((part) => part.entries.length > 0) }))
}

const SECTION_LABELS: Record<string, Record<Locale, string>> = {
  Feature: { en: 'New', 'zh-CN': '新功能' },
  Optimization: { en: 'Improved', 'zh-CN': '改进' },
  Fixed: { en: 'Fixed', 'zh-CN': '修复' },
  Security: { en: 'Security', 'zh-CN': '安全' },
  Changed: { en: 'Changed', 'zh-CN': '变更' },
}

/**
 * A heading such as `🚀 Feature` as `{ emoji: '🚀', label: '新功能' }`: the
 * emoji is kept from CHANGELOG.md, the word is translated. Unknown headings
 * keep their own word.
 */
export function sectionLabel(kind: string, locale: Locale): { emoji: string; label: string } {
  const match = kind.match(/^([^A-Za-z]*)(.*)$/)!
  const emoji = match[1].trim()
  const name = match[2].trim()
  return { emoji, label: SECTION_LABELS[name]?.[locale] ?? name }
}

function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (char) => `&#${char.charCodeAt(0)};`)
}

/**
 * Renders the little Markdown the changelog uses: `code` and [links](https://…).
 * Everything else is escaped, and only http(s) links survive.
 */
export function renderInline(text: string): string {
  const parts: string[] = []
  const pattern = /`([^`]+)`|\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g
  let last = 0
  for (const match of text.matchAll(pattern)) {
    parts.push(escapeHtml(text.slice(last, match.index)))
    if (match[1] !== undefined) parts.push(`<code>${escapeHtml(match[1])}</code>`)
    else parts.push(`<a href="${escapeHtml(match[3])}">${escapeHtml(match[2])}</a>`)
    last = (match.index ?? 0) + match[0].length
  }
  parts.push(escapeHtml(text.slice(last)))
  return parts.join('')
}
