import { useCallback, useEffect, useRef, useState } from 'react'
import type { Draft, DraftAttachmentSummary, DraftKind, InlineImage } from '@/generated/accounts'
import { clientFailure, clientFailureCode, type MailxClient, type ClientFailure } from '@/lib/mailx-client'
import { recipientsFromAddresses, toMailAddresses, type Recipient } from './address-parse'
import { draftHtmlIsEmpty } from './draft-html'

/** Typing pauses this long before the draft is written. */
export const AUTOSAVE_DELAY_MS = 1000
/** How long closing waits for the last write. */
export const FLUSH_LIMIT_MS = 2000

export type EditorFields = {
  accountId: string
  to: Recipient[]
  cc: Recipient[]
  bcc: Recipient[]
  subject: string
  bodyHtml: string
}

/**
 * What the footer says. `idle` shows nothing: a draft that was just opened, or one that is blank again
 * (a blank draft is not kept). `saving` only appears for saves slower than `SAVING_INDICATOR_DELAY_MS`.
 */
export type SaveStatus = 'idle' | 'saving' | 'saved' | 'failed'

export const SAVING_INDICATOR_DELAY_MS = 400

/** Why the editor cannot be used: the draft is not there, or something else broke. */
export type EditorProblem = ClientFailure

function fieldsOf(draft: Draft): EditorFields {
  return {
    accountId: draft.accountId,
    to: recipientsFromAddresses(draft.to),
    cc: recipientsFromAddresses(draft.cc),
    bcc: recipientsFromAddresses(draft.bcc),
    subject: draft.subject,
    bodyHtml: draft.bodyHtml,
  }
}

export function fieldsAreEmpty(fields: EditorFields) {
  return fields.to.length + fields.cc.length + fields.bcc.length === 0 && !fields.subject.trim() && draftHtmlIsEmpty(fields.bodyHtml)
}

/** Content identity for "changed since the window opened" (recipient chips compare by address). */
function contentKey(fields: EditorFields) {
  return JSON.stringify([
    fields.accountId, toMailAddresses(fields.to), toMailAddresses(fields.cc), toMailAddresses(fields.bcc),
    fields.subject, fields.bodyHtml,
  ])
}

type Notice = { kind: 'conflict' } | { kind: 'locked' }

/**
 * Owns one draft's editable copy and keeps it saved (sending.md section 7).
 *
 * - Saves 1 s after typing stops; only one save is ever in flight, and edits made
 *   meanwhile go out in the next round, so a slow write never reorders anything.
 * - Every save names the revision this editor last saw. On `conflict` the draft is
 *   reloaded from the core and the local edits are dropped (never merged silently);
 *   on `draftLocked` the editor turns read-only.
 * - `flush` is for closing and sending: it writes what is pending and reports
 *   whether everything is saved.
 */
export function useDraftEditor(client: MailxClient, draftId: string, onNotice?: (notice: Notice) => void) {
  const [fields, setFields] = useState<EditorFields | null>(null)
  const [kind, setKind] = useState<DraftKind>('new')
  const [problem, setProblem] = useState<EditorProblem | null>(null)
  const [status, setStatus] = useState<SaveStatus>('idle')
  const [readOnly, setReadOnly] = useState(false)
  const [gone, setGone] = useState(false)
  // Attachments change through their own commands (each bumps the revision), so they stay out of `EditorFields`
  // and autosave: an add or remove must never overwrite text typed meanwhile.
  const [attachments, setAttachments] = useState<DraftAttachmentSummary[]>([])
  const attachmentCount = useRef(0)
  const [attaching, setAttaching] = useState(false)
  // User edits and reloads withdraw send confirmation; an acknowledgement that
  // merely canonicalizes HTML must not invalidate the save it just confirmed.
  const [changeToken, setChangeToken] = useState(0)

  const latest = useRef<EditorFields | null>(null)
  // The content as it was when the window opened; "不保存" on an existing draft restores it.
  const baseline = useRef<EditorFields | null>(null)
  const removed = useRef(false)
  // Account removal is final, including while a load/save is still in flight.
  const detached = useRef(false)
  const revision = useRef(0)
  const dirty = useRef(false)
  const saving = useRef<Promise<void> | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const alive = useRef(true)
  const failed = useRef(false)
  const lockedRef = useRef(false)
  // An attachment command in flight; saves wait for it so two writers never name the same revision.
  const applyAttachments = useCallback((list: DraftAttachmentSummary[]) => {
    attachmentCount.current = list.length
    setAttachments(list)
  }, [])
  const mutating = useRef<Promise<unknown> | null>(null)
  const notice = useRef(onNotice)
  useEffect(() => { notice.current = onNotice })

  const apply = useCallback((draft: Draft) => {
    const next = fieldsOf(draft)
    latest.current = next
    revision.current = draft.revision
    dirty.current = false
    failed.current = false
    setFields(next)
    applyAttachments(draft.attachments)
    setChangeToken((value) => value + 1)
    setKind(draft.kind)
    setStatus('idle')
  }, [applyAttachments])

  const setLocked = useCallback((locked: boolean) => {
    lockedRef.current = locked
    setReadOnly(locked)
  }, [])

  const reload = useCallback(async () => {
    clearTimeout(timer.current)
    if (detached.current || !alive.current) return
    try {
      const draft = await client.getDraft(draftId)
      if (!alive.current || detached.current) return
      apply(draft)
      setProblem(null)
    } catch (cause) {
      if (!alive.current || detached.current) return
      if (clientFailureCode(cause) === 'notFound') setGone(true)
      else setProblem(clientFailure(cause))
    }
  }, [apply, client, draftId])

  useEffect(() => {
    alive.current = true
    // The first load, written as a promise chain rather than `reload()` so no state is set inside the effect body.
    client.getDraft(draftId).then(
      (draft) => {
        if (!alive.current || detached.current) return
        baseline.current ??= fieldsOf(draft)
        apply(draft)
        setProblem(null)
      },
      (cause) => {
        if (!alive.current || detached.current) return
        if (clientFailureCode(cause) === 'notFound') setGone(true)
        else setProblem(clientFailure(cause))
      },
    )
    return () => {
      alive.current = false
      clearTimeout(timer.current)
    }
  }, [apply, client, draftId])

  useEffect(() => client.onAccountRemoved((report) => {
    // The draft may have changed sender since this window opened. Only the
    // committed deletion report can tell us whether this draft was removed.
    if (!report.draftIds.includes(draftId)) return
    detached.current = true
    clearTimeout(timer.current)
    dirty.current = false
    failed.current = false
    setGone(true)
  }), [client, draftId])

  // The queue holds, releases or deletes this draft: follow it.
  useEffect(() => client.onDrafts((change) => {
    if (change.draftId !== draftId || detached.current) return
    if (change.change === 'locked') setLocked(true)
    else if (change.change === 'unlocked') {
      setLocked(false)
      void reload()
    } else if (change.change === 'deleted' && !removed.current) setGone(true)
  }), [client, draftId, reload, setLocked])

  const runSave = useCallback((): Promise<void> => {
    if (detached.current || !alive.current) return Promise.resolve()
    if (saving.current) return saving.current
    const round = (async () => {
      try {
        if (mutating.current) await mutating.current.catch(() => undefined)
        while (dirty.current && alive.current && !detached.current && !lockedRef.current) {
          const snapshot = latest.current
          if (!snapshot) return
          dirty.current = false
          // Local saves are usually instant: only a slow one earns a "saving" label.
          const slow = setTimeout(() => { if (alive.current && !detached.current) setStatus('saving') }, SAVING_INDICATOR_DELAY_MS)
          try {
            const saved = await client.updateDraft({
              id: draftId,
              expectedRevision: revision.current,
              accountId: snapshot.accountId,
              to: toMailAddresses(snapshot.to),
              cc: toMailAddresses(snapshot.cc),
              bcc: toMailAddresses(snapshot.bcc),
              subject: snapshot.subject,
              bodyHtml: snapshot.bodyHtml,
            })
            clearTimeout(slow)
            if (!alive.current || detached.current) return
            revision.current = saved.revision
            failed.current = false
            if (alive.current && !dirty.current) {
              // Only accept canonical content when no newer typing would be replaced.
              if (saved.bodyHtml !== snapshot.bodyHtml) {
                const canonical = { ...snapshot, bodyHtml: saved.bodyHtml }
                latest.current = canonical
                setFields(canonical)
              }
              const empty = saved.to.length + saved.cc.length + saved.bcc.length === 0 && !saved.subject.trim() && !saved.bodyText.trim()
              setStatus(empty ? 'idle' : 'saved')
            }
          } catch (cause) {
            clearTimeout(slow)
            if (!alive.current || detached.current) return
            const code = clientFailureCode(cause)
            if (code === 'conflict') {
              // Someone else saved a newer version: show it rather than overwrite it.
              dirty.current = false
              notice.current?.({ kind: 'conflict' })
              await reload()
              return
            }
            if (code === 'draftLocked') {
              dirty.current = false
              setLocked(true)
              setStatus('idle')
              notice.current?.({ kind: 'locked' })
              return
            }
            if (code === 'notFound') setGone(true)
            dirty.current = true
            failed.current = true
            if (alive.current) setStatus('failed')
            return
          }
        }
      } finally {
        saving.current = null
      }
    })()
    saving.current = round
    return round
  }, [client, draftId, reload, setLocked])

  const update = useCallback((patch: Partial<EditorFields>) => {
    if (detached.current || !alive.current || lockedRef.current || !latest.current) return
    const next = { ...latest.current, ...patch }
    latest.current = next
    dirty.current = true
    failed.current = false
    setFields(next)
    setChangeToken((value) => value + 1)
    // A blank draft is not kept, so don't leave "已保存" up while it waits to save.
    if (fieldsAreEmpty(next)) setStatus('idle')
    clearTimeout(timer.current)
    timer.current = setTimeout(() => { void runSave() }, AUTOSAVE_DELAY_MS)
  }, [runSave])

  /** Writes what is pending. `ok` means nothing unsaved remains. */
  const flush = useCallback(async (maxMs = FLUSH_LIMIT_MS): Promise<{ ok: boolean }> => {
    clearTimeout(timer.current)
    if (detached.current || !alive.current) return { ok: false }
    if (lockedRef.current) return { ok: true }
    let limit: ReturnType<typeof setTimeout> | undefined
    const timeout = new Promise<void>((resolve) => { limit = setTimeout(resolve, maxMs) })
    try {
      await Promise.race([(async () => {
        // A save already running may finish before the newest edits go out.
        while (!detached.current && alive.current && (saving.current || dirty.current)) {
          if (saving.current) await saving.current
          else {
            await runSave()
            if (failed.current) return
          }
          if (failed.current) return
        }
      })(), timeout])
    } finally {
      clearTimeout(limit)
    }
    return { ok: !detached.current && alive.current && !dirty.current && !saving.current && !failed.current }
  }, [runSave])

  /**
   * Runs one attachment command against the saved revision. Pending text is written first, and only the
   * revision and the attachment list are taken from the answer, never the text, so typing is not clobbered.
   * A failure rethrows after resyncing (earlier files of a multi-file add may have gone through).
   */
  const mutateAttachments = useCallback(async (command: (expectedRevision: number) => Promise<Draft | null | { draft: Draft; inserted: InlineImage[] }>): Promise<InlineImage[]> => {
    if (detached.current || !alive.current || lockedRef.current || mutating.current) return []
    setAttaching(true)
    const run = (async (): Promise<InlineImage[]> => {
      const { ok } = await flush()
      if (!ok) throw new DraftUnsavedError()
      try {
        const result = await command(revision.current)
        if (!result || !alive.current || detached.current) return []
        const draft = 'draft' in result ? result.draft : result
        revision.current = draft.revision
        applyAttachments(draft.attachments)
        return 'draft' in result ? result.inserted : []
      } catch (cause) {
        if (!alive.current || detached.current) throw cause
        const code = clientFailureCode(cause)
        if (code === 'conflict') await reload()
        else if (code === 'draftLocked') setLocked(true)
        else {
          try {
            const current = await client.getDraft(draftId)
            if (alive.current && !detached.current) {
              revision.current = current.revision
              applyAttachments(current.attachments)
            }
          } catch { /* the original failure is the one to show */ }
        }
        throw cause
      }
    })()
    mutating.current = run
    try {
      return await run
    } finally {
      mutating.current = null
      if (alive.current) setAttaching(false)
    }
  }, [applyAttachments, client, draftId, flush, reload, setLocked])

  /** Opens the host's file picker and attaches the choice. */
  const addAttachments = useCallback(
    () => mutateAttachments((expectedRevision) => client.addAttachments(draftId, expectedRevision)),
    [client, draftId, mutateAttachments],
  )
  /** Attaches the files the OS dropped on the window. */
  const addDroppedAttachments = useCallback(
    () => mutateAttachments((expectedRevision) => client.addDroppedAttachments(draftId, expectedRevision)),
    [client, draftId, mutateAttachments],
  )
  /** Opens the host's image picker; resolves with the images added (empty when cancelled). */
  const addInlineImages = useCallback(
    () => mutateAttachments((expectedRevision) => client.addInlineImages(draftId, expectedRevision)),
    [client, draftId, mutateAttachments],
  )
  /** Adds one pasted or dropped image. The body is flushed first, so earlier images stay referenced. */
  const addInlineImageBytes = useCallback(
    (file: File) => file.arrayBuffer().then((bytes) => mutateAttachments((expectedRevision) => client.addInlineImageBytes(draftId, expectedRevision, file.name, new Uint8Array(bytes)))),
    [client, draftId, mutateAttachments],
  )
  const removeAttachment = useCallback(
    (attachmentId: string) => mutateAttachments((expectedRevision) => client.removeAttachment({ draftId, attachmentId, expectedRevision })),
    [client, draftId, mutateAttachments],
  )

  /** Retries a failed save now. */
  const retry = useCallback(() => {
    if (detached.current || !alive.current || !dirty.current) return
    failed.current = false
    void runSave()
  }, [runSave])

  const remove = useCallback(async () => {
    if (detached.current || !alive.current) return
    clearTimeout(timer.current)
    dirty.current = false
    removed.current = true
    if (saving.current) await saving.current
    if (detached.current || !alive.current) return
    try {
      await client.deleteDraft(draftId)
    } catch (cause) {
      if (detached.current || !alive.current) return
      removed.current = false
      throw cause
    }
  }, [client, draftId])

  /** Whether the content differs from what the window opened with. */
  const changed = useCallback(() => {
    const now = latest.current
    const before = baseline.current
    return !!now && !!before && contentKey(now) !== contentKey(before)
  }, [])

  /** Writes the opening content back (after any save in flight), at the latest revision. */
  const restoreBaseline = useCallback(async () => {
    if (detached.current || !alive.current) return
    clearTimeout(timer.current)
    dirty.current = false
    if (saving.current) await saving.current
    if (detached.current || !alive.current) return
    const before = baseline.current
    if (!before || lockedRef.current || !changed()) return
    const saved = await client.updateDraft({
      id: draftId,
      expectedRevision: revision.current,
      accountId: before.accountId,
      to: toMailAddresses(before.to),
      cc: toMailAddresses(before.cc),
      bcc: toMailAddresses(before.bcc),
      subject: before.subject,
      bodyHtml: before.bodyHtml,
    })
    if (detached.current || !alive.current) return
    revision.current = saved.revision
  }, [changed, client, draftId])

  return {
    fields,
    kind,
    problem,
    status,
    readOnly,
    gone,
    attachments,
    attaching,
    addAttachments,
    addDroppedAttachments,
    addInlineImages,
    addInlineImageBytes,
    removeAttachment,
    changeToken,
    update,
    flush,
    retry,
    reload,
    remove,
    changed,
    restoreBaseline,
    markLocked: () => setLocked(true),
    revision: () => revision.current,
    isEmpty: () => (latest.current ? fieldsAreEmpty(latest.current) && attachmentCount.current === 0 : false),
    latest: () => latest.current,
    isAvailable: () => alive.current && !detached.current,
  }
}

/** Attachments were not changed because pending text could not be written first. */
export class DraftUnsavedError extends Error {}

export type DraftEditor = ReturnType<typeof useDraftEditor>
