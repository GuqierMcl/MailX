import { Plugin, PluginKey, type EditorState } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import type { DocRange } from './compose-ai'

/**
 * What writing help shows in the document (ai-chat-ui.md 5.3), all display only:
 * the text being rewritten, dimmed; the card right after its block; and the
 * "let AI draft" hint in an empty body. None of it is part of the document, so
 * nothing reaches the saved HTML.
 */
export type ComposeAiMarks = {
  /** The text the card is about; `null` for a draft that inserts. Dimmed while set. */
  source: DocRange | null
  /** Where the card sits: the end of a top-level block. */
  anchor: number
}

type PluginState = { marks: ComposeAiMarks | null; hint: number | null; stale: boolean }
export type ComposeAiMeta = { marks?: ComposeAiMarks | null; hint?: number | null }

export const composeAiKey = new PluginKey<PluginState>('mailComposeAi')

/** Whether a mapping changed anything inside `[from, to)`, inserting at its edges aside. */
function touches(maps: readonly { forEach: (f: (oldStart: number, oldEnd: number) => void) => void; map: (pos: number, assoc?: number) => number }[], range: DocRange) {
  let { from, to } = range
  let touched = false
  for (const map of maps) {
    map.forEach((oldStart, oldEnd) => {
      if (oldStart < to && oldEnd > from) touched = true
      else if (oldStart === oldEnd && oldStart > from && oldStart < to) touched = true
    })
    if (touched) return true
    from = map.map(from, 1)
    to = map.map(to, -1)
  }
  return false
}

/**
 * `card` and `hint` are this editor's own elements; React renders into them. An
 * edit inside the source makes the state `stale` and calls `onStale` once.
 */
export function composeAiPlugin(card: HTMLElement, hint: HTMLElement, onStale: () => void) {
  let reported = false
  return new Plugin<PluginState>({
    key: composeAiKey,
    state: {
      init: () => ({ marks: null, hint: null, stale: false }),
      apply(tr, value) {
        const meta = tr.getMeta(composeAiKey) as ComposeAiMeta | undefined
        let { marks, hint: hintAt, stale } = value
        if (meta && 'marks' in meta) { marks = meta.marks ?? null; stale = false }
        else if (marks && tr.docChanged) {
          if (marks.source && touches(tr.mapping.maps, marks.source)) stale = true
          const source = marks.source ? { from: tr.mapping.map(marks.source.from, 1), to: tr.mapping.map(marks.source.to, -1) } : null
          marks = { source, anchor: tr.mapping.map(marks.anchor, 1) }
        }
        if (meta && 'hint' in meta) hintAt = meta.hint ?? null
        else if (hintAt !== null && tr.docChanged) hintAt = null
        return { marks, hint: hintAt, stale }
      },
    },
    props: {
      decorations(state: EditorState) {
        const value = composeAiKey.getState(state)
        if (!value) return null
        const decorations: Decoration[] = []
        const { marks } = value
        if (marks) {
          if (marks.source && marks.source.from < marks.source.to) {
            decorations.push(Decoration.inline(marks.source.from, marks.source.to, { class: 'mail-ai-source' }))
          }
          // Before any other widget at the same place (a reply's folded quote starts right there).
          decorations.push(Decoration.widget(Math.min(marks.anchor, state.doc.content.size), card, { side: -1, key: 'compose-ai-card', ignoreSelection: true, stopEvent: () => true }))
        }
        if (value.hint !== null && value.hint <= state.doc.content.size) {
          decorations.push(Decoration.widget(value.hint, hint, { side: 1, key: 'compose-ai-hint', ignoreSelection: true }))
        }
        return decorations.length ? DecorationSet.create(state.doc, decorations) : null
      },
    },
    view: () => ({
      update(view) {
        const value = composeAiKey.getState(view.state)
        if (value?.stale && !reported) { reported = true; onStale() }
        if (!value?.stale) reported = false
      },
    }),
  })
}
