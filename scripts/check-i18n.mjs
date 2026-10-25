import { readFile, readdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const directory = fileURLToPath(new URL('../locales/', import.meta.url))
const languages = ['zh-CN', 'en']
const errors = []

function flatten(value, prefix = '', output = {}) {
  for (const [key, entry] of Object.entries(value)) {
    const name = prefix ? `${prefix}.${key}` : key
    if (typeof entry === 'string' && entry.trim()) output[name] = entry
    else if (entry && typeof entry === 'object' && !Array.isArray(entry)) flatten(entry, name, output)
    else errors.push(`Invalid message: ${name}`)
  }
  return output
}

function messages(catalog, language) {
  const result = new Map()
  const plurals = new Intl.PluralRules(language).resolvedOptions().pluralCategories
  for (const [key, value] of Object.entries(flatten(catalog))) {
    const plural = /_(zero|one|two|few|many|other)$/.exec(key)
    const name = plural ? key.slice(0, plural.index) : key
    const entry = result.get(name) ?? { parameters: new Set(), branches: new Set() }
    if (plural) {
      entry.branches.add(plural[1])
      entry.parameters.add('count')
      if (!plurals.includes(plural[1])) errors.push(`${language}:${key}: unexpected plural branch`)
    }
    for (const match of value.matchAll(/{{\s*([a-zA-Z][a-zA-Z0-9]*)\s*}}/g)) entry.parameters.add(match[1])
    if (/{{|}}/.test(value.replace(/{{\s*[a-zA-Z][a-zA-Z0-9]*\s*}}/g, ''))) errors.push(`${language}:${key}: malformed interpolation`)
    result.set(name, entry)
  }
  for (const [key, entry] of result) {
    if (entry.branches.size) for (const branch of plurals) {
      if (!entry.branches.has(branch)) errors.push(`${language}:${key}: missing ${branch} plural branch`)
    }
  }
  return result
}

const names = await Promise.all(languages.map(async (language) => (await readdir(path.join(directory, language))).filter((name) => name.endsWith('.json')).sort()))
if (JSON.stringify(names[0]) !== JSON.stringify(names[1])) errors.push('Languages have different namespaces')
let total = 0
for (const name of names[0]) {
  const catalogs = await Promise.all(languages.map(async (language) => JSON.parse(await readFile(path.join(directory, language, name), 'utf8'))))
  const sets = catalogs.map((catalog, i) => messages(catalog, languages[i]))
  for (const key of new Set([...sets[0].keys(), ...sets[1].keys()])) {
    const [zh, en] = sets.map((set) => set.get(key))
    if (!zh || !en) errors.push(`${name}:${key}: missing translation`)
    else if (JSON.stringify([...zh.parameters].sort()) !== JSON.stringify([...en.parameters].sort())) errors.push(`${name}:${key}: different parameters`)
    else if (Boolean(zh.branches.size) !== Boolean(en.branches.size)) errors.push(`${name}:${key}: inconsistent plural message`)
    total += 1
  }
  // The Rust adapter supports this explicit, bounded subset of resource syntax.
  if (name === 'notifications.json' || name === 'composeTemplates.json') {
    const required = name === 'notifications.json'
      ? { noSubject: [], newMail: ['count'], newMailFrom: ['count', 'senders', 'more'], moreSenders: [], sent: [], failed: [], unknown: [], test: [] }
      : { newMailTitle: [], imageAlt: ['alt'], replyIntro: ['date', 'sender'], forwardHeader: ['date', 'sender', 'subject', 'recipients'] }
    for (const [key, parameters] of Object.entries(required)) for (let i = 0; i < sets.length; i++) {
      const entry = sets[i].get(key)
      if (!entry || JSON.stringify([...entry.parameters].sort()) !== JSON.stringify([...parameters].sort())) errors.push(`${languages[i]}:${name}:${key}: Rust resource contract mismatch`)
    }
  }
}
if (errors.length) {
  console.error(errors.join('\n'))
  process.exitCode = 1
} else console.log(`i18n resources valid: ${total} messages across ${names[0].length} namespaces and ${languages.length} languages.`)
