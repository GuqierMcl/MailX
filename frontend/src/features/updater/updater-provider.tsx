import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { toast } from '@/components/ui/toast'
import type { MailxClient } from '@/lib/mailx-client'
import { announcements, shouldAnnounce } from './announce'
import { UpdateDialog } from './update-dialog'
import type { UpdateSnapshot } from './types'
import { UpdaterContext } from './updater-context'

/**
 * Owns the update state the main window sees: it subscribes to host events,
 * turns transitions into toasts and mounts the modal. Composer windows never
 * mount this.
 */
export function UpdaterProvider({ client, children }: { client: MailxClient; children: ReactNode }) {
  useTranslation()
  const [snapshot, setSnapshot] = useState<UpdateSnapshot>({ canInstall: true, phase: 'idle' })
  const [open, setOpen] = useState(false)
  const [checked, setChecked] = useState(false)
  const latest = useRef<UpdateSnapshot>(snapshot)
  const openRef = useRef(false)
  const announced = useRef(new Set<string>())
  const manual = useRef(false)

  useEffect(() => {
    openRef.current = open
  }, [open])

  const apply = useCallback((next: UpdateSnapshot, manualAnswer = false) => {
    const previous = latest.current
    latest.current = next
    setSnapshot(next)
    if (previous.phase === 'checking' && next.phase !== 'checking') setChecked(true)
    if (manualAnswer) setChecked(true)
    for (const notice of announcements(previous, next, manualAnswer)) {
      if (!shouldAnnounce(notice, announced.current, openRef.current)) continue
      if (notice.kind === 'available') {
        announced.current.add(notice.version)
        toast.info(t('updater:availableToast', { version: notice.version }), {
          timeout: 0,
          action: { label: t('updater:viewUpdate'), onClick: () => setOpen(true) },
        })
      } else {
        toast.info(t('updater:downloadedToast', { version: notice.version }), {
          timeout: 0,
          action: { label: t('updater:restartNow'), onClick: () => void client.restartToUpdate().catch(() => undefined) },
        })
      }
    }
    // A manual check is answered directly: the modal when there is a version
    // to talk about, the About page's status line otherwise. Never a toast.
    if (manualAnswer && next.phase === 'available') setOpen(true)
  }, [client])

  useEffect(() => {
    let disposed = false
    client.getUpdateState().then(
      (state) => { if (!disposed) apply(state) },
      () => undefined,
    )
    const stop = client.onUpdateState((state) => {
      if (disposed) return
      // The host's own events during a manual check are that check's answer.
      const answer = manual.current && state.phase !== 'checking'
      if (answer) manual.current = false
      apply(state, answer)
    })
    return () => { disposed = true; stop() }
  }, [client, apply])

  const check = useCallback(async () => {
    manual.current = true
    try {
      const next = await client.checkForUpdates()
      if (manual.current) {
        manual.current = false
        apply(next, true)
      }
    } catch {
      // The host reports a failed check as a state too; a lost one still ends here.
      manual.current = false
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
