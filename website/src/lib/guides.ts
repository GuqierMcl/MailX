/**
 * Maps a guide file to its Starlight id, which is also its URL:
 *
 *   zh-CN/index.md     → zh-CN/docs
 *   en/add-account.md  → en/docs/add-account
 */
export function guideId(entry: string): string {
  const match = entry.match(/^(zh-CN|en)\/(.+?)\.mdx?$/)
  if (!match) throw new Error(`Guide outside zh-CN/ or en/: ${entry}`)
  const [, locale, path] = match
  const page = path === 'index' ? '' : `/${path.replace(/\/index$/, '').toLowerCase()}`
  return `${locale}/docs${page}`
}
