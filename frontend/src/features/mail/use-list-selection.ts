import { useCallback, useMemo, useState } from 'react'
import type { EmailSelection, EmailSummary, SelectedEmail } from '@/generated/accounts'

/** The most messages one batch action may name, as the host enforces. */
export const SELECTION_LIMIT = 500

/** The fields a batch action needs, from a listed row or from "select all". */
export function toSelected({ id, accountId, folderId, isRead, isStarred, archived, localOnly }: SelectedEmail | EmailSummary): SelectedEmail {
  return { id, accountId, folderId, isRead, isStarred, archived, localOnly }
}

type State = {
  scope: string
  active: boolean
  /** Bumped on every entry and exit, so a late "select all" cannot land in another session. */
  session: number
  /** In the order the messages were picked; values may be stale for listed rows. */
  picked: Map<string, SelectedEmail>
  /** The row a Shift+click range starts from: the last one clicked. */
  anchor: string | null
  /** "Select all" picked everything in scope (until something is deselected). */
  all: boolean
  /** The 500 cap left messages out. */
  truncated: boolean
  /** The rows the previous render listed, to notice the ones that disappear. */
  listed: EmailSummary[]
}

const EMPTY = new Map<string, SelectedEmail>()

function initial(scope: string, listed: EmailSummary[], session = 0): State {
  return { scope, active: false, session, picked: EMPTY, anchor: null, all: false, truncated: false, listed }
}

/** Adds rows up to the cap; reports whether anything was left out. */
function add(picked: Map<string, SelectedEmail>, rows: Array<SelectedEmail | EmailSummary>) {
  const next = new Map(picked)
  let truncated = false
  for (const row of rows) {
    // A sender's group row stands for many messages and is never picked itself.
    if ('senderGroup' in row && row.senderGroup) continue
    if (next.has(row.id)) continue
    if (next.size >= SELECTION_LIMIT) {
      truncated = true
      break
    }
    next.set(row.id, toSelected(row))
  }
  return { next, truncated }
}

/**
 * Multi-select state for the mail list. It never depends on which rows are
 * mounted (the list is virtualized) and can hold messages that are not loaded
 * yet, which "select all" returns from the host. A new scope (mailbox, search,
 * filter) clears it and leaves the mode; rows that drop out of the list
 * (synced away, deleted) silently leave the selection.
 */
export function useListSelection(emails: EmailSummary[], scope: string) {
  const [state, setState] = useState<State>(() => initial(scope, emails))
  // Adjusted while rendering (React's "information from previous renders"), so
  // the list and the selection never disagree for a frame.
  let current = state
  if (state.scope !== scope) {
    current = initial(scope, emails, state.session + 1)
    setState(current)
  } else if (state.listed !== emails) {
    const listed = new Set(emails.map((email) => email.id))
    const gone = state.listed.filter((email) => !listed.has(email.id) && state.picked.has(email.id))
    let picked = state.picked
    if (gone.length > 0) {
      picked = new Map(picked)
      for (const email of gone) picked.delete(email.id)
    }
    current = { ...state, listed: emails, picked }
    setState(current)
  }

  const items = useMemo(() => {
    const live = new Map(emails.map((email) => [email.id, email]))
    return [...current.picked.values()].map((item) => {
      const row = live.get(item.id)
      return row ? toSelected(row) : item
    })
  }, [current.picked, emails])

  /** Cmd/Ctrl/Shift+click outside the mode: only that row, never the opened message. */
  const start = useCallback((email: EmailSummary) => setState((s) => email.senderGroup ? s : ({
    ...s, active: true, session: s.session + 1, picked: new Map([[email.id, toSelected(email)]]), anchor: email.id, all: false, truncated: false,
  })), [])
  const enter = useCallback(() => setState((s) => s.active ? s : ({
    ...s, active: true, session: s.session + 1, picked: EMPTY, anchor: null, all: false, truncated: false,
  })), [])
  const exit = useCallback(() => setState((s) => s.active ? ({
    ...s, active: false, session: s.session + 1, picked: EMPTY, anchor: null, all: false, truncated: false,
  }) : s), [])
  const toggle = useCallback((email: EmailSummary) => setState((s) => {
    if (email.senderGroup) return s
    if (s.picked.has(email.id)) {
      const picked = new Map(s.picked)
      picked.delete(email.id)
      return { ...s, picked, anchor: email.id, all: false }
    }
    const { next, truncated } = add(s.picked, [email])
    return { ...s, picked: next, anchor: email.id, truncated: s.truncated || truncated }
  }), [])
  /** Shift+click: every row from the anchor to this one, in list order. */
  const extend = useCallback((email: EmailSummary, rows: EmailSummary[]) => setState((s) => {
    if (email.senderGroup) return s
    const from = s.anchor === null ? -1 : rows.findIndex((row) => row.id === s.anchor)
    const to = rows.findIndex((row) => row.id === email.id)
    if (to < 0) return s
    const range = from < 0 ? [email] : rows.slice(Math.min(from, to), Math.max(from, to) + 1)
    const { next, truncated } = add(s.picked, range)
    return { ...s, picked: next, anchor: from < 0 ? email.id : s.anchor, truncated: s.truncated || truncated }
  }), [])
  const deselectAll = useCallback(() => setState((s) => ({ ...s, picked: EMPTY, all: false, truncated: false })), [])
  /** Loads every message in scope (up to the cap) from the host; ignored if the scope or session changed meanwhile. */
  const { scope: currentScope, session } = current
  const selectAll = useCallback(async (load: () => Promise<EmailSelection>) => {
    const selection = await load()
    setState((s) => {
      if (s.scope !== currentScope || s.session !== session || !s.active) return s
      const { next, truncated } = add(EMPTY, selection.items)
      return { ...s, picked: next, all: true, truncated: selection.truncated || truncated }
    })
  }, [currentScope, session])
  /** Takes messages out, e.g. the ones a batch action finished. */
  const drop = useCallback((ids: string[]) => setState((s) => {
    if (!ids.some((id) => s.picked.has(id))) return s
    const picked = new Map(s.picked)
    for (const id of ids) picked.delete(id)
    return { ...s, picked }
  }), [])
  /** Keeps only these messages selected (the ones an action could not finish). */
  const keep = useCallback((ids: string[]) => setState((s) => {
    const wanted = new Set(ids)
    return { ...s, picked: new Map([...s.picked].filter(([id]) => wanted.has(id))), all: false }
  }), [])

  const picked = current.picked
  const isSelected = useCallback((id: string) => picked.has(id), [picked])
  // "Everything" stops being true once a listed row is outside the selection,
  // e.g. new mail that arrived after selecting all.
  const allSelected = current.active && current.all && picked.size > 0 && emails.every((email) => email.senderGroup || picked.has(email.id))

  return {
    active: current.active,
    count: picked.size,
    items,
    isSelected,
    allSelected,
    truncated: current.active && current.truncated,
    start, enter, exit, toggle, extend, deselectAll, selectAll, drop, keep,
  }
}

export type ListSelection = ReturnType<typeof useListSelection>
