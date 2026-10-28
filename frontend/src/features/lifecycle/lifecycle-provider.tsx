import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { errorText } from '@/i18n/errors'
import type { CloseAction, LifecycleClient, LifecycleSnapshot } from './lifecycle-client'
import { LifecycleContext } from './lifecycle-context'
import { prepareMainWindowQuit } from './prepare-quit'

/** Kept outside startup admission so a failed mailbox startup never prevents closing. */
export function LifecycleProvider({ client, children }: { client: LifecycleClient | null; children: ReactNode }) {
  const queryClient = useQueryClient()
  const [snapshot, setSnapshot] = useState<LifecycleSnapshot | null>(null)
  const latest = useRef<LifecycleSnapshot | null>(null)
  const [quitFailure, setQuitFailure] = useState<{ generation: number; error: unknown } | null>(null)
  useEffect(() => {
    if (!client) return
    let disposed = false
    let stop: (() => void) | undefined
    const accept = (next: LifecycleSnapshot) => {
      if (disposed || (latest.current && next.revision < latest.current.revision)) return
      latest.current = next
      setSnapshot(next)
      if (next.quitting) setQuitFailure((current) => current?.generation === next.quitRequestId ? current : null)
    }
    void client.onSnapshot(accept).then(async (unsubscribe) => {
      if (disposed) { unsubscribe(); return }
      stop = unsubscribe
      accept(await client.getSnapshot())
    }).catch(() => undefined)
    return () => { disposed = true; stop?.() }
  }, [client])
  const quitting = snapshot?.quitting ?? false
  const generation = snapshot?.quitRequestId ?? null
  useEffect(() => {
    if (!client || !quitting || generation === null) return
    const controller = new AbortController()
    const current = () => !controller.signal.aborted && latest.current?.quitting && latest.current.quitRequestId === generation
    void (async () => {
      try {
        await prepareMainWindowQuit(queryClient, controller.signal)
        if (current()) await client.confirmQuit(generation)
      } catch (cause) {
        if (!current()) return
        setQuitFailure({ generation, error: cause })
        // Bind cancellation too: an old failed write cannot cancel a newer quit.
        try { await client.cancelQuit(generation) } catch { /* The host's deadline still cancels an unconfirmed quit. */ }
      }
    })()
    return () => controller.abort()
  }, [client, generation, queryClient, quitting])

  useEffect(() => {
    if (!quitting) return
    // Portaled settings menus are outside the inert main wrapper. Also stop
    // their input and global shortcuts while the quit dialog owns interaction.
    const block = (event: Event) => {
      if (event.target instanceof Element && event.target.closest('[data-quit-controls]')) return
      event.preventDefault()
      event.stopImmediatePropagation()
    }
    const events = ['click', 'pointerdown', 'keydown', 'beforeinput', 'paste', 'cut', 'drop', 'submit']
    for (const name of events) document.addEventListener(name, block, true)
    return () => { for (const name of events) document.removeEventListener(name, block, true) }
  }, [quitting])
  const value = useMemo(() => client ? { client, snapshot } : null, [client, snapshot])
  return <LifecycleContext.Provider value={value}>
    <div className="contents" inert={quitting} data-main-interaction>{children}</div>
    {client && snapshot?.closeRequestId !== null && snapshot?.closeRequestId !== undefined && !snapshot.quitting &&
      <CloseChoiceDialog key={snapshot.closeRequestId} client={client} snapshot={snapshot} requestId={snapshot.closeRequestId} />}
    {client && snapshot && !quitFailure && (snapshot.quitting || snapshot.quitTimedOut) &&
      <QuitProgress key={snapshot.quitting ? `quitting-${generation}` : 'timedOut'} client={client} generation={generation} timedOut={snapshot.quitTimedOut && !snapshot.quitting} />}
    {quitFailure && <QuitFailure error={quitFailure.error} onClose={() => setQuitFailure(null)} />}
  </LifecycleContext.Provider>
}

function CloseChoiceDialog({ client, snapshot, requestId }: { client: LifecycleClient; snapshot: LifecycleSnapshot; requestId: number }) {
  const { t } = useTranslation('settings')
  const [remember, setRemember] = useState(false)
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const [error, setError] = useState<unknown>(null)
  const [resolved, setResolved] = useState(false)
  const resolve = async (action: CloseAction) => {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(true)
    setError(null)
    try {
      await client.resolveClose({ requestId, action, remember: action !== 'cancel' && remember })
      setResolved(true)
    } catch (cause) {
      setError(cause)
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }
  const mac = snapshot.platform === 'macos'
  return <Dialog open={!resolved} onOpenChange={(open) => { if (!open && !busy) void resolve('cancel') }}>
    <DialogContent className="sm:max-w-lg" showCloseButton={!busy}>
      <DialogHeader>
        <DialogTitle>{t('closePromptTitle')}</DialogTitle>
        <DialogDescription>{t(!snapshot.backgroundAvailable ? 'trayUnavailable' : mac ? 'closePromptMacDescription' : 'closePromptDescription')}</DialogDescription>
      </DialogHeader>
      {error !== null && <p role="alert" className="text-sm text-destructive">{errorText(error)}</p>}
      <label className="flex cursor-pointer items-center gap-2 text-sm">
        <Checkbox checked={remember} disabled={busy} onCheckedChange={(checked) => setRemember(checked === true)} />
        {t('rememberCloseChoice')}
      </label>
      <DialogFooter>
        <Button variant="ghost" disabled={busy} onClick={() => void resolve('cancel')}>{t('closeCancel')}</Button>
        {snapshot.backgroundAvailable && <Button disabled={busy} onClick={() => void resolve('background')}>{t(mac ? 'closeBackgroundMac' : 'closeBackgroundTray')}</Button>}
        <Button variant="outline" disabled={busy} onClick={() => void resolve('quit')}>{t('closeQuit')}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
}

/** A DOM dialog stays above an existing Settings modal without activating the native window. */
function QuitProgress({ client, generation, timedOut }: { client: LifecycleClient; generation: number | null; timedOut: boolean }) {
  const { t } = useTranslation('settings')
  const [dismissed, setDismissed] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const cancel = async () => {
    setBusy(true)
    try { await client.cancelQuit(generation ?? undefined) } catch (cause) { setError(cause) } finally { setBusy(false) }
  }
  if (dismissed) return null
  return <Dialog open onOpenChange={(open) => { if (!open && !busy) { if (timedOut) setDismissed(true); else void cancel() } }}>
    <DialogContent showCloseButton={false} data-quit-controls>
      <DialogHeader>
        <DialogTitle>{t(timedOut ? 'quitCancelledTitle' : 'quittingTitle')}</DialogTitle>
        <DialogDescription>{t(timedOut ? 'quitTimedOut' : 'quitting')}</DialogDescription>
      </DialogHeader>
      {error !== null && <p role="alert" className="text-destructive">{errorText(error)}</p>}
      <DialogFooter>
        <Button variant="outline" disabled={busy} onClick={() => timedOut ? setDismissed(true) : void cancel()}>
          {t(timedOut ? 'dismissQuitNotice' : 'cancelQuit')}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
}

function QuitFailure({ error, onClose }: { error: unknown; onClose: () => void }) {
  const { t } = useTranslation('settings')
  return <Dialog open onOpenChange={(open) => { if (!open) onClose() }}>
    <DialogContent data-quit-controls>
      <DialogHeader>
        <DialogTitle>{t('quitSaveFailedTitle')}</DialogTitle>
        <DialogDescription>{t('quitSaveFailedDescription')}</DialogDescription>
      </DialogHeader>
      <p role="alert" className="text-sm text-destructive">{errorText(error)}</p>
      <DialogFooter><Button variant="outline" onClick={onClose}>{t('dismissQuitNotice')}</Button></DialogFooter>
    </DialogContent>
  </Dialog>
}
