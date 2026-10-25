import { Extension, Mark, type Command } from '@tiptap/core'
import {
  isMailColor, isMailHighlight, isMailTextAlign,
  type MailColor, type MailHighlight, type MailTextAlign,
} from './mail-body-format'

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    mailTextAlign: {
      setMailTextAlign: (alignment: MailTextAlign) => ReturnType
      unsetMailTextAlign: () => ReturnType
    }
    mailColor: {
      setMailColor: (color: MailColor) => ReturnType
      unsetMailColor: () => ReturnType
    }
    mailHighlight: {
      setMailHighlight: (highlight: MailHighlight) => ReturnType
      unsetMailHighlight: () => ReturnType
    }
  }
}

function alignText(alignment: MailTextAlign): Command {
  return ({ tr, dispatch }) => {
    if (alignment !== 'left' && !isMailTextAlign(alignment)) return false
    const textAlign = alignment === 'left' ? null : alignment
    const storedMarks = tr.storedMarks
    let applicable = false
    let changed = false
    // Walk the selected text blocks, including paragraphs inside lists and
    // quotes. Container nodes themselves never receive alignment attributes.
    for (const range of tr.selection.ranges) {
      tr.doc.nodesBetween(range.$from.pos, range.$to.pos, (node, position) => {
        if (node.type.name !== 'paragraph' && node.type.name !== 'heading') return
        applicable = true
        if (dispatch && node.attrs.textAlign !== textAlign) {
          tr.setNodeMarkup(position, undefined, { ...node.attrs, textAlign })
          changed = true
        }
      })
    }
    // setNodeMarkup clears pending typing marks. Preserve the chain's current
    // marks, including an explicit empty set from a preceding clear command.
    if (changed && tr.selection.empty) tr.setStoredMarks(storedMarks)
    return applicable
  }
}

/** The CSP-safe equivalent of TextAlign; deliberately has no keyboard shortcuts. */
export const MailTextAlignment = Extension.create({
  name: 'mailTextAlign',
  addGlobalAttributes() {
    return [{
      types: ['paragraph', 'heading'],
      attributes: {
        textAlign: {
          default: null,
          parseHTML: (element) => {
            const alignment = element.getAttribute('data-align')
            return isMailTextAlign(alignment) ? alignment : null
          },
          renderHTML: (attributes) => isMailTextAlign(attributes.textAlign) ? { 'data-align': attributes.textAlign } : {},
        },
      },
    }]
  },
  addCommands() {
    return {
      setMailTextAlign: (alignment) => alignText(alignment),
      unsetMailTextAlign: () => alignText('left'),
    }
  },
})

/** A fixed palette mark. It neither imports CSS colors nor generates styles. */
export const MailTextColor = Mark.create({
  name: 'mailColor',
  addAttributes() {
    return {
      color: {
        default: null,
        parseHTML: (element) => {
          const color = element.getAttribute('data-color')
          return isMailColor(color) ? color : null
        },
        renderHTML: (attributes) => isMailColor(attributes.color) ? { 'data-color': attributes.color } : {},
      },
    }
  },
  parseHTML() {
    return [{ tag: 'span[data-color]', getAttrs: (element) => isMailColor(element.getAttribute('data-color')) ? null : false }]
  },
  renderHTML({ HTMLAttributes }) {
    return ['span', HTMLAttributes, 0]
  },
  addCommands() {
    return {
      setMailColor: (color) => ({ commands }) => isMailColor(color) && commands.setMark(this.name, { color }),
      unsetMailColor: () => ({ commands }) => commands.unsetMark(this.name),
    }
  },
})

export const MailTextHighlight = Mark.create({
  name: 'mailHighlight',
  addAttributes() {
    return {
      highlight: {
        default: null,
        parseHTML: (element) => {
          const highlight = element.getAttribute('data-highlight')
          return isMailHighlight(highlight) ? highlight : null
        },
        renderHTML: (attributes) => isMailHighlight(attributes.highlight) ? { 'data-highlight': attributes.highlight } : {},
      },
    }
  },
  parseHTML() {
    return [{ tag: 'mark[data-highlight]', getAttrs: (element) => isMailHighlight(element.getAttribute('data-highlight')) ? null : false }]
  },
  renderHTML({ HTMLAttributes }) {
    return ['mark', HTMLAttributes, 0]
  },
  addCommands() {
    return {
      setMailHighlight: (highlight) => ({ commands }) => isMailHighlight(highlight) && commands.setMark(this.name, { highlight }),
      unsetMailHighlight: () => ({ commands }) => commands.unsetMark(this.name),
    }
  },
})
