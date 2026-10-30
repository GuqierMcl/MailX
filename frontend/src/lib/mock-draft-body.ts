import { isMailColor, isMailHighlight, isMailTextAlign } from '@/features/composer/mail-body-format'

const BLOCKS = new Set(['P', 'H1', 'H2', 'H3'])
const ALLOWED = new Set(['P', 'BR', 'STRONG', 'B', 'EM', 'I', 'U', 'S', 'STRIKE', 'DEL', 'A', 'UL', 'OL', 'LI', 'BLOCKQUOTE', 'H1', 'H2', 'H3', 'SPAN', 'MARK', 'IMG'])
const DROP_CONTENT = new Set(['SCRIPT', 'STYLE', 'IFRAME', 'OBJECT', 'EMBED', 'TEMPLATE', 'SVG', 'MATH', 'NOSCRIPT', 'HEAD', 'TITLE'])

function safeHref(value: string): string | null {
  const href = value.trim()
  if (!/^(https?:\/\/|mailto:)/i.test(href) || Array.from(href).some((character) => character.charCodeAt(0) <= 32 || character.charCodeAt(0) === 127)) return null
  try {
    const url = new URL(href)
    return (url.protocol === 'mailto:' ? url.pathname.length > 0 : url.hostname.length > 0) ? href : null
  } catch {
    return null
  }
}

/** Browser-preview approximation only. Production HTML/text are owned by Rust. */
export function mockDraftBody(html: string): { bodyHtml: string; bodyText: string } {
  const template = document.createElement('template')
  template.innerHTML = html
  for (const node of Array.from(template.content.querySelectorAll('*'))) {
    if (node.namespaceURI !== 'http://www.w3.org/1999/xhtml' || DROP_CONTENT.has(node.tagName)) {
      node.remove()
      continue
    }
    const href = node.tagName === 'A' ? safeHref(node.getAttribute('href') ?? '') : null
    const start = node.tagName === 'OL' ? node.getAttribute('start') : null
    const align = BLOCKS.has(node.tagName) ? node.getAttribute('data-align') : null
    const cid = node.tagName === 'IMG' && node.getAttribute('src')?.startsWith('cid:') ? node.getAttribute('src') : null
    const alt = cid ? node.getAttribute('alt') : null
    const color = node.tagName === 'SPAN' ? node.getAttribute('data-color') : null
    const highlight = node.tagName === 'MARK' ? node.getAttribute('data-highlight') : null
    for (const attribute of Array.from(node.attributes)) {
      node.removeAttribute(attribute.name)
    }
    if (href) node.setAttribute('href', href)
    if (cid) node.setAttribute('src', cid)
    if (cid && alt) node.setAttribute('alt', alt)
    if (start && /^-?\d+$/.test(start) && Number.isSafeInteger(Number(start))) node.setAttribute('start', String(Number(start)))
    if (isMailTextAlign(align)) node.setAttribute('data-align', align)
    if (isMailColor(color)) node.setAttribute('data-color', color)
    if (isMailHighlight(highlight)) node.setAttribute('data-highlight', highlight)
    if (!ALLOWED.has(node.tagName) || (node.tagName === 'SPAN' && !isMailColor(color)) || (node.tagName === 'MARK' && !isMailHighlight(highlight)) || (node.tagName === 'IMG' && !cid)) {
      node.replaceWith(...node.childNodes)
    }
  }
  const text = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? ''
    const inside = Array.from(node.childNodes).map(text).join('')
    if (!(node instanceof HTMLElement)) return inside
    if (node.tagName === 'BR') return '\n'
    if (node.tagName === 'A') {
      const href = node.getAttribute('href')
      return href && inside !== href ? `${inside} <${href}>` : inside
    }
    if (node.tagName === 'BLOCKQUOTE') return inside.replace(/\n$/, '').split('\n').map((line) => line.trim() ? `> ${line}` : '').join('\n') + '\n'
    if (node.tagName === 'LI' && inside.trim()) {
      const parent = node.parentElement
      const number = Number(parent?.getAttribute('start') ?? 1) + (parent ? Array.from(parent.children).indexOf(node) : 0)
      return `${parent?.tagName === 'OL' ? `${number}. ` : '- '}${inside.replace(/\n$/, '')}\n`
    }
    return inside + (BLOCKS.has(node.tagName) ? '\n' : '')
  }
  return { bodyHtml: template.innerHTML, bodyText: text(template.content).replace(/\n$/, '') }
}

export function mockPlainToHtml(text: string): string {
  return text.replace(/\r\n?/g, '\n').split('\n').map((line) => `<p>${line.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')}</p>`).join('')
}
