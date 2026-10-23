/**
 * Composer windows load the same bundle as the main window with
 * `?composer=<draftId>` (see docs/architecture/sending.md section 2). The
 * host fixes the window label to `composer-<draftId>` and only lets that
 * window touch its own draft; this helper just tells the page which role to render.
 */
const DRAFT_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

export type WindowRole = { kind: 'main' } | { kind: 'composer'; draftId: string }

/** The draft id in `?composer=`; `null` for anything but a canonical lowercase UUID. */
export function composerDraftId(search: string): string | null {
  const value = new URLSearchParams(search).get('composer')
  return value !== null && DRAFT_ID.test(value) ? value : null
}

export function windowRole(search: string = globalThis.location?.search ?? ''): WindowRole {
  const draftId = composerDraftId(search)
  return draftId ? { kind: 'composer', draftId } : { kind: 'main' }
}

/** The query string the host appends to `index.html` for a draft's window. */
export function composerSearch(draftId: string): string {
  return `?composer=${draftId}`
}
