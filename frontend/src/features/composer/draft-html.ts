/** A local hint while typing; the core derives the authoritative plain text on save. */
export function draftHtmlIsEmpty(html: string): boolean {
  const template = document.createElement('template')
  template.innerHTML = html
  // An untouched default signature block does not count as user-authored body content.
  template.content.querySelectorAll('div[data-mailx-signature]').forEach((node) => node.remove())
  return !template.content.textContent?.trim() && !template.content.querySelector('a[href], img[src^="cid:"]')
}

/** Extracts the first signature block's id and normalized inner HTML, or `null` if none exists. */
export function extractSignatureBlock(html: string): { signatureId: string; innerHtml: string } | null {
  const template = document.createElement('template')
  template.innerHTML = html
  const block = template.content.querySelector('div[data-mailx-signature]')
  if (!block) return null
  return {
    signatureId: block.getAttribute('data-mailx-signature') ?? '',
    innerHtml: block.innerHTML.trim(),
  }
}
