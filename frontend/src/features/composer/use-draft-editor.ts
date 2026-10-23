import { useCallback, useEffect, useRef, useState } from 'react'
import type { Draft, DraftKind } from '@/generated/accounts'
import { clientFailure, clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { recipientsFromAddresses, toMailAddresses, type Recipient } from './address-parse'

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
  bodyText: string
}

export type SaveStatus = 'saved' | 'saving' | 'failed'

/** Why the editor cannot be used: the draft is not there, or something else broke. */
export type EditorProblem = { message: string }

function fieldsOf(draft: Draft): EditorFields {
  return {
    accountId: draft.accountId,
    to: recipientsFromAddresses(draft.to),
    cc: recipientsFromAddresses(draft.cc),
    bcc: recipientsFromAddresses(draft.bcc),
    subject: draft.subject,
    bodyText: draft.bodyText,
  }
}

export function fieldsAreEmpty(fields: EditorFields) {
  return fields.to.length + fields.cc.length + fields.bcc.length === 0 && !fields.subject.trim() && !fields.bodyText.trim()
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
  const [status, setStatus] = useState<SaveStatus>('saved')
  const [readOnly, setReadOnly] = useState(false)
  const [gone, setGone] = useState(false)

  const latest = useRef<EditorFields | null>(null)
  const revision = useRef(0)
  const dirty = useRef(false)
  const saving = useRef<Promise<void> | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const alive = useRef(true)
  const failed = useRef(false)
  const lockedRef = useRef(false)
  const notice = useRef(onNotice)
  useEffect(() => { notice.current = onNotice })

  const apply = useCallback((draft: Draft) => {
    const next = fieldsOf(draft)
    latest.current = next
    revision.current = draft.revision
    dirty.current = false
    failed.current = false
    setFields(next)
    setKind(draft.kind)
    setStatus('saved')
  }, [])

  const setLocked = useCallback((locked: boolean) => {
    lockedRef.current = locked
    setReadOnly(locked)
  }, [])

  const reload = useCallback(async () => {
    clearTimeout(timer.current)
    try {
      const draft = await client.getDraft(draftId)
      if (!alive.current) return
      apply(draft)
      setProblem(null)
    } catch (cause) {
      if (!alive.current) return
      if (clientFailureCode(cause) === 'notFound') setGone(true)
      else setProblem({ message: clientFailure(cause).message })
    }
  }, [apply, client, draftId])

  useEffect(() => {
    alive.current = true
    // The first load, written as a promise chain rather than `reload()` so no state is set inside the effect body.
    client.getDraft(draftId).then(
      (draft) => { if (alive.current) { apply(draft); setProblem(null) } },
      (cause) => {
        if (!alive.current) return
        if (clientFailureCode(cause) === 'notFound') setGone(true)
        else setProblem({ message: clientFailure(cause).message })
      },
    )
    return () => {
      alive.current = false
      clearTimeout(timer.current)
    }
  }, [apply, client, draftId])

  // The queue holds, releases or deletes this draft: follow it.
  useEffect(() => client.onDrafts((change) => {
    if (change.draftId !== draftId) return
    if (change.change === 'locked') setLocked(true)
    else if (change.change === 'unlocked') {
      setLocked(false)
      void reload()
    } else setGone(true)
  }), [client, draftId, reload, setLocked])

  const runSave = useCallback((): Promise<void> => {
    if (saving.current) return saving.current
    const round = (async () => {
      try {
        while (dirty.current && alive.current && !lockedRef.current) {
          const snapshot = latest.current
          if (!snapshot) return
          dirty.current = false
          setStatus('saving')
          try {
            const saved = await client.updateDraft({
              id: draftId,
              expectedRevision: revision.current,
              accountId: snapshot.accountId,
              to: toMailAddresses(snapshot.to),
              cc: toMailAddresses(snapshot.cc),
              bcc: toMailAddresses(snapshot.bcc),
              subject: snapshot.subject,
              bodyText: snapshot.bodyText,
            })
            revision.current = saved.revision
            failed.current = false
            if (alive.current && !dirty.current) setStatus('saved')
          } catch (cause) {
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
              setStatus('saved')
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
    if (lockedRef.current || !latest.current) return
    const next = { ...latest.current, ...patch }
    latest.current = next
    dirty.current = true
    failed.current = false
    setFields(next)
    setStatus('saving')
    clearTimeout(timer.current)
    timer.current = setTimeout(() => { void runSave() }, AUTOSAVE_DELAY_MS)
  }, [runSave])

  /** Writes what is pending. `ok` means nothing unsaved remains. */
  const flush = useCallback(async (maxMs = FLUSH_LIMIT_MS): Promise<{ ok: boolean }> => {
    clearTimeout(timer.current)
    if (lockedRef.current) return { ok: true }
    let limit: ReturnType<typeof setTimeout> | undefined
    const timeout = new Promise<void>((resolve) => { limit = setTimeout(resolve, maxMs) })
    try {
      await Promise.race([(async () => {
        // A save already running may finish before the newest edits go out.
        while (saving.current || dirty.current) {
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
    return { ok: !dirty.current && !saving.current && !failed.current }
  }, [runSave])

  /** Retries a failed save now. */
  const retry = useCallback(() => {
    if (!dirty.current) return
    failed.current = false
    void runSave()
  }, [runSave])

  const remove = useCallback(async () => {
    clearTimeout(timer.current)
    dirty.current = false
    if (saving.current) await saving.current
    await client.deleteDraft(draftId)
  }, [client, draftId])

  return {
    fields,
    kind,
    problem,
    status,
    readOnly,
    gone,
    update,
    flush,
    retry,
    reload,
    remove,
    markLocked: () => setLocked(true),
    revision: () => revision.current,
    isEmpty: () => (latest.current ? fieldsAreEmpty(latest.current) : false),
    latest: () => latest.current,
  }
}

export type DraftEditor = ReturnType<typeof useDraftEditor>
