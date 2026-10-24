import type { EditorOptions } from '@tiptap/core'
import HardBreak from '@tiptap/extension-hard-break'
import Link from '@tiptap/extension-link'
import { Fragment, Slice } from '@tiptap/pm/model'
import StarterKit from '@tiptap/starter-kit'

/** Keep the editing schema in step with the core's outgoing HTML whitelist. */
export function allowedMailLink(value: string): boolean {
  if (!/^(https?:\/\/|mailto:)/i.test(value) || Array.from(value).some((character) => character.charCodeAt(0) <= 32 || character.charCodeAt(0) === 127)) return false
  try {
    const url = new URL(value)
    return url.protocol === 'mailto:' ? url.pathname.length > 0 : url.hostname.length > 0
  } catch {
    return false
  }
}

// HardBreak's default Mod-Enter would consume the composer's send shortcut.
const MailHardBreak = HardBreak.extend({
  addKeyboardShortcuts() {
    return { 'Shift-Enter': () => this.editor.commands.setHardBreak() }
  },
})

const MailLink = Link.extend({
  addAttributes() {
    return { href: { default: null } }
  },
}).configure({
  openOnClick: false,
  autolink: true,
  linkOnPaste: true,
  markdownLinks: false,
  defaultProtocol: 'https',
  HTMLAttributes: { target: null, rel: null, class: null },
  isAllowedUri: allowedMailLink,
  shouldAutoLink: allowedMailLink,
})

/**
 * Clipboard HTML is never mounted. Remove active content and all incidental
 * attributes before ProseMirror parses it into the small editing schema. In
 * particular, pasted CSS must not become formatting via a mark's style rule.
 * The core independently sanitizes every save; this is the editing boundary.
 */
export function filterClipboardHtml(html: string): string {
  // A template's inert document never loads image/iframe resources while we
  // inspect them; DOMParser.parseFromString can fetch those before filtering.
  const template = document.createElement('template')
  template.innerHTML = html
  template.content.querySelectorAll('script,style,iframe,object,embed,svg,math,template,noscript,img,video,audio,source,link,meta').forEach((element) => element.remove())
  for (const element of template.content.querySelectorAll('*')) {
    const href = element.tagName === 'A' ? element.getAttribute('href') : null
    const start = element.tagName === 'OL' ? element.getAttribute('start') : null
    for (const attribute of Array.from(element.attributes)) element.removeAttribute(attribute.name)
    if (href && allowedMailLink(href.trim())) element.setAttribute('href', href.trim())
    if (start && /^-?\d+$/.test(start) && Number.isSafeInteger(Number(start))) element.setAttribute('start', String(Number(start)))
  }
  return template.innerHTML
}

export function mailEditorOptions(): Partial<EditorOptions> {
  return {
    injectCSS: false,
    // Saved mail uses pre-wrap, so indentation, repeated spaces and tabs are
    // meaningful text. Do not collapse them when reopening a core-owned draft.
    parseOptions: { preserveWhitespace: 'full' },
    extensions: [
      StarterKit.configure({
        heading: false,
        code: false,
        codeBlock: false,
        horizontalRule: false,
        hardBreak: false,
        link: false,
        // These add no useful behavior to our text-only schema and the drop
        // cursor would draw an element with inline styles under the strict CSP.
        dropcursor: false,
        gapcursor: false,
        trailingNode: false,
      }),
      MailHardBreak,
      MailLink,
    ],
    enableInputRules: ['bulletList', 'orderedList', 'blockquote'],
    // Plain URLs are intentional; Markdown emphasis and link syntax are not.
    enablePasteRules: ['link'],
    editorProps: {
      attributes: { role: 'textbox', 'aria-label': '正文', 'aria-multiline': 'true', class: 'mail-body-content' },
      transformPastedHTML: filterClipboardHtml,
      clipboardTextParser(text, _context, _plain, view) {
        const { schema } = view.state
        const paragraphs = text.split(/\r\n?|\n/).map((line) => schema.nodes.paragraph.create(null, line ? schema.text(line) : null))
        return Slice.maxOpen(Fragment.fromArray(paragraphs))
      },
    },
  }
}
