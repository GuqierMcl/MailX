import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { toast } from '@/components/ui/toast'
import type { MailxClient } from '@/lib/mailx-client'
import { announcementKey, announcements, shouldAnnounce } from './announce'
import { restartBlockedText } from './details'
import { UpdateDialog } from './update-dialog'
import { IDLE_UPDATE, updateVersion, type UpdateSnapshot } from './types'
import { UpdaterContext } from './updater-context'

type ManualCheck = {
  seenChecking: boolean
  answered: boolean
  sequence: number
  initial: UpdateSnapshot
}

/**
 * Owns the update state the main window sees: it subscribes to host events,
 * turns transitions into toasts and mounts the modal. Composer windows never
 * mount this.
 */
export function UpdaterProvider({ client, children }: { client: MailxClient; children: ReactNode }) {
  useTranslation()
  const [snapshot, setSnapshot] = useState<UpdateSnapshot>(IDLE_UPDATE)
  const [open, setOpen] = useState(false)
  const [checked, setChecked] = useState(false)
  const latest = useRef<UpdateSnapshot>(snapshot)
  const openRef = useRef(false)
  const announced = useRef(new Set<string>())
  const manual = useRef<ManualCheck | null>(null)
  const eventSequence = useRef(0)

  useEffect(() => {
    openRef.current = open
  }, [open])

  const apply = useCallback((next: UpdateSnapshot, manualAnswer = false, suppressNotice = false) => {
    const previous = latest.current
    latest.current = next
    setSnapshot(next)
    if (previous.phase === 'checking' && next.phase !== 'checking') setChecked(true)
    if (manualAnswer) setChecked(true)
    // The host may have started or finished an automatic download before the
    // check returns. Every result with a version can answer a manual check.
    if (manualAnswer && updateVersion(next)) {
      openRef.current = true
      setOpen(true)
    }
    for (const notice of announcements(previous, next, manualAnswer || suppressNotice)) {
      const show = shouldAnnounce(notice, announced.current, openRef.current)
      announced.current.add(announcementKey(notice))
      if (!show) continue
      if (notice.kind === 'available') {
        toast.info(t('updater:availableToast', { version: notice.version }), {
          timeout: 0,
          action: { label: t('updater:viewUpdate'), onClick: () => setOpen(true) },
        })
      } else if (notice.kind === 'ready') {
        toast.info(t('updater:downloadedToast', { version: notice.version }), {
          timeout: 0,
          action: {
            label: t('updater:restartNow'),
            onClick: () => void client.restartToUpdate().catch((error: unknown) => {
              toast.error(restartBlockedText(error) ?? t('updater:restartFailed'))
            }),
          },
        })
      } else {
        toast.error(t('updater:installFailed'), {
          timeout: 0,
          action: { label: t('updater:viewUpdate'), onClick: () => setOpen(true) },
        })
      }
    }
  }, [client])

  useEffect(() => {
    let disposed = false
    const stop = client.onUpdateState((state) => {
      if (disposed) return
      eventSequence.current += 1
      // Settings refresh can emit an unchanged snapshot before the check starts.
      // Only a transition out of Checking answers this request through events.
      const request = manual.current
      if (request && state.phase === 'checking') request.seenChecking = true
      const answer = Boolean(request?.seenChecking && !request.answered && state.phase !== 'checking')
      if (request && answer) request.answered = true
      apply(state, answer, request !== null)
    })
    const sequence = eventSequence.current
    client.getUpdateState().then(
      (state) => { if (!disposed && sequence === eventSequence.current) apply(state) },
      () => undefined,
    )
    return () => { disposed = true; stop() }
  }, [client, apply])

  const check = useCallback(async () => {
    if (manual.current) return
    eventSequence.current += 1
    const request: ManualCheck = { seenChecking: false, answered: false, sequence: eventSequence.current, initial: latest.current }
    manual.current = request
    try {
      const next = await client.checkForUpdates()
      if (manual.current === request && !request.answered) {
        const observed = latest.current
        // Ready may return immediately without Checking. Keep a newer version
        // event or a same-phase settings change, but don't mistake an unchanged
        // pre-check Available/Idle event for the command's eventual answer.
        const newerResult = eventSequence.current > request.sequence && updateVersion(observed) && (
          observed.phase !== request.initial.phase || updateVersion(observed) !== updateVersion(request.initial)
          || (observed.phase === next.phase && updateVersion(observed) === updateVersion(next))
        )
        apply(newerResult ? observed : next, true)
      }
    } catch {
      // The host reports a failed check as a state too; a lost one still ends here.
    } finally {
      if (manual.current === request) manual.current = null
    }
  }, [client, apply])

  const value = useMemo(() => ({ snapshot, checked, check, openDialog: () => setOpen(true) }), [snapshot, checked, check])

  return (
    <UpdaterContext.Provider value={value}>
      {children}
      <UpdateDialog client={client} snapshot={snapshot} open={open} onOpenChange={setOpen} />
    </UpdaterContext.Provider>
  )
}
