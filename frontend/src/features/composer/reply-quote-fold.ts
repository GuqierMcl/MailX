import { Extension } from '@tiptap/core'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import { Plugin, PluginKey, Selection, type EditorState, type Transaction } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import { replyQuoteSpan } from './reply-quote'

/** The folded part of the document: `[from, to)` and the start of each hidden top-level node. */
export type FoldedQuote = { from: number; to: number; nodes: number[] }

export const replyQuoteFoldKey = new PluginKey<FoldedQuote | null>('mailReplyQuoteFold')

/** The reply quote as the shared rule finds it in this document. */
export function findReplyQuote(doc: ProseMirrorNode): FoldedQuote | null {
  const blocks: Parameters<typeof replyQuoteSpan>[0][number][] = []
  const positions: number[] = []
  doc.forEach((node, offset) => {
    blocks.push({ quote: node.type.name === 'blockquote', paragraph: node.type.name === 'paragraph', empty: !node.textContent.trim() })
    positions.push(offset)
  })
  const span = replyQuoteSpan(blocks)
  if (!span) return null
  return { from: positions[span.start], to: positions[span.end] + doc.child(span.end).nodeSize, nodes: positions.slice(span.start, span.end + 1) }
}

type Options = {
  /** Replies fold, other drafts never do. */
  enabled: boolean
}

type Storage = {
  /**
   * The "···" button's container, one per editor; the React editor renders the
   * button into it (`editor.storage.mailReplyQuoteFold.widget`). It must never
   * be shared: two views (React StrictMode briefly makes two editors) moving
   * one element back and forth each see a DOM change, re-read and redraw it,
   * and so loop for ever.
   */
  widget: HTMLElement | null
}

declare module '@tiptap/core' {
  interface Storage {
    mailReplyQuoteFold: { widget: HTMLElement | null }
  }
}

/**
 * Display-only folding of a reply's quoted original (sending.md section 17.5).
 * Decorations hide the nodes; the document, and so the saved HTML, never
 * changes. Once unfolded (the button, or the selection moving into the quote)
 * it stays unfolded for this editor; an edit that changes the quote's shape
 * unfolds it too, so a different block can never become hidden.
 */
export const ReplyQuoteFold = Extension.create<Options, Storage>({
  name: 'mailReplyQuoteFold',
  addOptions: () => ({ enabled: false }),
  addStorage: () => ({ widget: null }),
  addProseMirrorPlugins() {
    const { enabled } = this.options
    const widget = enabled ? document.createElement('div') : null
    if (widget) {
      widget.className = 'mail-quote-fold'
      widget.contentEditable = 'false'
    }
    this.storage.widget = widget
    return [new Plugin<FoldedQuote | null>({
      key: replyQuoteFoldKey,
      state: {
        init: (_, state) => (enabled && widget ? findReplyQuote(state.doc) : null),
        apply(tr, folded, _old, state) {
          if (!folded || tr.getMeta(replyQuoteFoldKey) === 'expand') return null
          let current = folded
          if (tr.docChanged) {
            const found = findReplyQuote(state.doc)
            if (!found || found.from !== tr.mapping.map(folded.from, 1) || found.to !== tr.mapping.map(folded.to, -1)) return null
            current = found
          }
          const { from, to } = state.selection
          return from < current.to && to > current.from ? null : current
        },
      },
      props: {
        decorations(state) {
          const folded = replyQuoteFoldKey.getState(state)
          if (!folded || !widget) return null
          return DecorationSet.create(state.doc, [
            Decoration.widget(folded.from, () => widget, { side: -1, key: 'mail-quote-fold', ignoreSelection: true, stopEvent: () => true }),
            ...folded.nodes.map((position) => Decoration.node(position, position + state.doc.nodeAt(position)!.nodeSize, { class: 'mail-quote-folded' })),
          ])
        },
        // Hidden nodes are skipped by the browser's own caret movement, so the
        // arrow keys step into the quote here; the selection then unfolds it.
        handleKeyDown(view, event) {
          const folded = replyQuoteFoldKey.getState(view.state)
          if (!folded || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey || event.isComposing) return false
          const forward = event.key === 'ArrowDown' || event.key === 'ArrowRight'
          if (!forward && event.key !== 'ArrowUp' && event.key !== 'ArrowLeft') return false
          const { $head } = view.state.selection
          if (!$head.parent.isTextblock) return false
          const direction = event.key === 'ArrowDown' ? 'down' : event.key === 'ArrowUp' ? 'up' : forward ? 'forward' : 'backward'
          if (!view.endOfTextblock(direction)) return false
          const edge = forward ? $head.after() : $head.before()
          const next = Selection.findFrom(view.state.doc.resolve(edge), forward ? 1 : -1, true)
          if (!next || next.from <= folded.from || next.from >= folded.to) return false
          view.dispatch(view.state.tr.setSelection(next).scrollIntoView())
          return true
        },
      },
    })]
  },
})

/** Unfolds the quote; nothing happens when it is not folded. */
export function expandReplyQuote(view: { state: EditorState; dispatch: (tr: Transaction) => void }) {
  if (replyQuoteFoldKey.getState(view.state)) view.dispatch(view.state.tr.setMeta(replyQuoteFoldKey, 'expand').setMeta('addToHistory', false))
}
