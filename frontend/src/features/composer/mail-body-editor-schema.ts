import { t } from '@/i18n'
import { generateHTML, type EditorOptions } from '@tiptap/core'
import HardBreak from '@tiptap/extension-hard-break'
import Link from '@tiptap/extension-link'
import { Markdown } from '@tiptap/markdown'
import { DOMParser, Fragment, Slice } from '@tiptap/pm/model'
import { Plugin } from '@tiptap/pm/state'
import StarterKit from '@tiptap/starter-kit'
import { isMailTextAlign } from './mail-body-format'
import { MailTextAlignment, MailTextColor, MailTextHighlight } from './mail-body-format-extensions'

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
  parseMarkdown(token, helpers) {
    const content = helpers.parseInline(token.tokens || [])
    return allowedMailLink(token.href ?? '') ? helpers.applyMark('link', content, { href: token.href }) : content
  },
}).configure({
  openOnClick: false,
  autolink: true,
  linkOnPaste: true,
  markdownLinks: true,
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
    const alignment = /^(P|H[1-3])$/.test(element.tagName) ? element.getAttribute('data-align') : null
    for (const attribute of Array.from(element.attributes)) element.removeAttribute(attribute.name)
    if (href && allowedMailLink(href.trim())) element.setAttribute('href', href.trim())
    if (start && /^-?\d+$/.test(start) && Number.isSafeInteger(Number(start))) element.setAttribute('start', String(Number(start)))
    if (isMailTextAlign(alignment)) element.setAttribute('data-align', alignment)
    // Even valid palette names in external HTML are discarded. Color/highlight
    // only return through core-owned saved content or our formatting commands.
  }
  return template.innerHTML
}

// The official extension supplies parsing/serialization, but no clipboard
// plugin. This bridge uses its parser and the same mail paste boundary as HTML.
const MailMarkdown = Markdown.extend({
  addProseMirrorPlugins() {
    const editor = this.editor
    return [new Plugin({
      props: {
        clipboardTextParser(text, _context, plain, view) {
          if (!plain) {
            const content = editor.markdown!.parse(text)
            if (content.content?.length) {
              const template = document.createElement('template')
              template.innerHTML = filterClipboardHtml(generateHTML(content, editor.extensionManager.baseExtensions))
              const slice = DOMParser.fromSchema(view.state.schema).parseSlice(template.content, { preserveWhitespace: 'full' })
              if (slice.size > 0) return slice
            }
          }
          // Keep the original plain-paste behavior and don't replace a selection
          // with nothing when Markdown only contains unsupported nodes.
          const paragraphs = text.split(/\r\n?|\n/).map((line) => view.state.schema.nodes.paragraph.create(null, line ? view.state.schema.text(line) : null))
          return Slice.maxOpen(Fragment.fromArray(paragraphs))
        },
      },
    })]
  },
})

export function mailEditorOptions(): Partial<EditorOptions> {
  return {
    injectCSS: false,
    // Saved mail uses pre-wrap, so indentation, repeated spaces and tabs are
    // meaningful text. Do not collapse them when reopening a core-owned draft.
    parseOptions: { preserveWhitespace: 'full' },
    extensions: [
      StarterKit.configure({
        heading: { levels: [1, 2, 3] },
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
      MailMarkdown,
      MailTextAlignment,
      MailTextColor,
      MailTextHighlight,
    ],
    // Markdown has already been parsed. Don't reapply mark paste rules to its
    // output (for example, interpreting escaped emphasis a second time).
    enablePasteRules: false,
    editorProps: {
      attributes: { role: 'textbox', 'aria-label': t('composer:field.body'), 'aria-multiline': 'true', class: 'mail-body-content' },
      transformPastedHTML: filterClipboardHtml,
    },
  }
}
