import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from '@/components/ui/toast'
import type { ComposerController } from '@/features/composer/composer'
import { InlineDraftUnsaved, registerQuitSave } from '@/features/lifecycle/prepare-quit'
import type { MailxClient } from '@/lib/mailx-client'
import { composeKeys } from '@/lib/query-client'

/** The draft the main window's reader is writing. `fresh`: created for it, as a composer window's `fresh=1`. */
export type InlineDraft = { draftId: string; fresh: boolean; key: number }

export type InlineComposer = {
  current: InlineDraft | null
  /** Saving before it goes away (or moves to a window); the editor is inert meanwhile. */
  busy: boolean
  controllerRef: RefObject<ComposerController | null>
  /** The draft on screen right now, read without waiting for a render. */
  draftId: () => string | null
  /** Shows a draft. Only after `leave()` succeeded: there is one inline composer at a time. */
  show: (draftId: string, fresh: boolean) => void
  /**
   * Saves and closes (sending.md section 17.7): a blank draft is deleted quietly,
   * anything else stays in Drafts with a "已存为草稿" toast. False when the save
   * failed; the composer then stays open with the failure in its bottom bar.
   */
  leave: () => Promise<boolean>
  /** Unfolds the reader for the draft already shown, keeping its editor as it is. */
  reveal: () => void
  /** Writes what is pending and hands the draft to its own composer window. */
  popOut: () => Promise<boolean>
  /** Gone without saving: it was sent or discarded from inside. */
  close: () => void
}

/**
 * Owns the main window's inline composer: which draft it shows and how it goes
 * away. `onShow` runs whenever a draft appears (the reader unfolds for it);
 * `onContinue` is the toast's "继续编辑", which reopens per the current setting.
 */
export function useInlineComposer(
  client: MailxClient,
  { onShow, onContinue }: { onShow: () => void; onContinue: (draftId: string) => void },
): InlineComposer {
  const queryClient = useQueryClient()
  const [current, setCurrentState] = useState<InlineDraft | null>(null)
  const [busy, setBusy] = useState(false)
  const currentRef = useRef<InlineDraft | null>(null)
  const controllerRef = useRef<ComposerController | null>(null)
  // Deleted or removed from elsewhere: nothing is left to save.
  const gone = useRef(false)
  const pending = useRef<Promise<boolean> | null>(null)
  const seq = useRef(0)
  const callbacks = useRef({ onShow, onContinue })
  useEffect(() => { callbacks.current = { onShow, onContinue } })

  const setCurrent = useCallback((next: InlineDraft | null) => {
    currentRef.current = next
    gone.current = false
    setCurrentState(next)
  }, [])
  // Only the draft a step started with may be closed by it.
  const closeIf = useCallback((draft: InlineDraft) => {
    if (currentRef.current === draft) setCurrent(null)
  }, [setCurrent])

  // One leave or pop-out at a time; a second request joins the one running.
  const exclusive = useCallback((step: (draft: InlineDraft) => Promise<boolean>): Promise<boolean> => {
    if (pending.current) return pending.current
    const draft = currentRef.current
    if (!draft) return Promise.resolve(true)
    setBusy(true)
    const running = step(draft).catch(() => false)
    pending.current = running
    void running.finally(() => {
      if (pending.current !== running) return
      pending.current = null
      setBusy(false)
    })
    return running
  }, [])

  const leave = useCallback(() => exclusive(async (draft) => {
    const controller = controllerRef.current
    if (!controller || gone.current || controller.isReadOnly()) {
      closeIf(draft)
      return true
    }
    const failed = () => {
      toast.error(t('mail:inlineComposeSaveFailed'), { description: t('mail:inlineComposeSaveFailedDescription') })
      return false
    }
    // Left before its fields arrived: nothing was typed. A new draft is as blank as it
    // was created and is deleted; an existing one is untouched and just closes.
    if (!controller.isLoaded()) {
      if (draft.fresh) {
        try {
          await controller.discard()
        } catch {
          return failed()
        }
      }
      closeIf(draft)
      return true
    }
    let ok: boolean
    try {
      ok = (await controller.flush()).ok
    } catch {
      ok = false
    }
    if (currentRef.current !== draft) return true
    if (!ok) return failed()
    if (controller.isEmpty()) {
      try {
        await controller.discard()
      } catch {
        return failed()
      }
      closeIf(draft)
      return true
    }
    // A draft reopened and left as it was needs no word about it.
    const saved = draft.fresh || controller.hasChanges()
    closeIf(draft)
    void queryClient.invalidateQueries({ queryKey: composeKeys.drafts() })
    if (saved) {
      toast.success(t('mail:inlineComposeSaved'), {
        action: { label: t('mail:inlineComposeContinue'), onClick: () => callbacks.current.onContinue(draft.draftId) },
      })
    }
    return true
  }), [closeIf, exclusive, queryClient])

  const popOut = useCallback(() => exclusive(async (draft) => {
    const controller = controllerRef.current
    if (controller && !gone.current && !controller.isReadOnly()) {
      let ok: boolean
      try {
        ok = (await controller.flush()).ok
      } catch {
        ok = false
      }
      if (currentRef.current !== draft) return false
      if (!ok) {
        toast.error(t('mail:inlineComposeSaveFailed'), { description: t('mail:inlineComposeSaveFailedDescription') })
        return false
      }
    }
    try {
      // `fresh` travels along, so closing the window unsaved still deletes a new draft.
      await client.openComposer(draft.draftId, draft.fresh)
    } catch (cause) {
      toast.error(t('mail:inlineComposePopOutFailed'), { description: errorText(cause) })
      return false
    }
    closeIf(draft)
    return true
  }), [client, closeIf, exclusive])

  const show = useCallback((draftId: string, fresh: boolean) => {
    seq.current += 1
    setCurrent({ draftId, fresh, key: seq.current })
    callbacks.current.onShow()
  }, [setCurrent])

  const reveal = useCallback(() => callbacks.current.onShow(), [])
  const close = useCallback(() => setCurrent(null), [setCurrent])
  const draftId = useCallback(() => currentRef.current?.draftId ?? null, [])

  // The host keeps agent tools (MCP, the assistant) from changing or sending the
  // draft on screen, as it does for a composer window. Reported on every change,
  // including the first mount, so a reloaded page clears what the last one left.
  const shownId = current?.draftId ?? null
  useEffect(() => {
    client.setInlineComposer(shownId).catch(() => undefined)
  }, [client, shownId])
  useEffect(() => () => {
    client.setInlineComposer(null).catch(() => undefined)
  }, [client])

  // The account and its drafts are gone: close at once, nothing to save.
  useEffect(() => client.onAccountRemoved((report) => {
    const draft = currentRef.current
    if (draft && report.draftIds.includes(draft.draftId)) setCurrent(null)
  }), [client, setCurrent])
  useEffect(() => client.onDrafts((change) => {
    if (change.change === 'deleted' && change.draftId === currentRef.current?.draftId) gone.current = true
  }), [client])

  // An explicit quit writes the draft like a composer window does; a failure
  // cancels the quit and keeps the editor. Blank drafts are kept until then
  // (desktop-lifecycle.md): the quit may still be cancelled elsewhere.
  useEffect(() => registerQuitSave(queryClient, async () => {
    const controller = controllerRef.current
    if (!currentRef.current || !controller || gone.current || controller.isReadOnly()) return
    let ok: boolean
    try {
      ok = (await controller.flush()).ok
    } catch {
      ok = false
    }
    if (!ok) throw new InlineDraftUnsaved()
  }), [queryClient])

  return useMemo(() => ({ current, busy, controllerRef, draftId, show, reveal, leave, popOut, close }), [busy, close, current, draftId, leave, popOut, reveal, show])
}
