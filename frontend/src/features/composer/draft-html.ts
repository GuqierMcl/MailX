/** A local hint while typing; the core derives the authoritative plain text on save. */
export function draftHtmlIsEmpty(html: string): boolean {
  const template = document.createElement('template')
  template.innerHTML = html
  return !template.content.textContent?.trim() && !template.content.querySelector('a[href]')
}
