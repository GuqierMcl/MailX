/** Browser-preview approximation only. Production HTML/text are owned by Rust. */
export function mockDraftBody(html: string): { bodyHtml: string; bodyText: string } {
  const template = document.createElement('template')
  template.innerHTML = html
  const allowed = new Set(['P', 'BR', 'STRONG', 'B', 'EM', 'I', 'U', 'S', 'STRIKE', 'DEL', 'A', 'UL', 'OL', 'LI', 'BLOCKQUOTE'])
  for (const node of Array.from(template.content.querySelectorAll('*'))) {
    if (['SCRIPT', 'STYLE', 'IFRAME', 'OBJECT', 'TEMPLATE', 'SVG', 'MATH'].includes(node.tagName)) {
      node.remove()
      continue
    }
    for (const attribute of Array.from(node.attributes)) {
      const link = node.tagName === 'A' && attribute.name === 'href' && /^(https?:\/\/|mailto:)/i.test(attribute.value.trim())
      const start = node.tagName === 'OL' && attribute.name === 'start' && /^\d+$/.test(attribute.value)
      if (!link && !start) node.removeAttribute(attribute.name)
    }
    if (!allowed.has(node.tagName)) node.replaceWith(...node.childNodes)
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
    return inside + (node.tagName === 'P' ? '\n' : '')
  }
  return { bodyHtml: template.innerHTML, bodyText: text(template.content).replace(/\n$/, '') }
}

export function mockPlainToHtml(text: string): string {
  return text.replace(/\r\n?/g, '\n').split('\n').map((line) => `<p>${line.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')}</p>`).join('')
}
