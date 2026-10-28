import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { Account } from '@/generated/accounts'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { toast } from '@/components/ui/toast'
import { Island } from '@/features/shell/island'
import { TitleBar } from '@/features/window-chrome/title-bar'
import type { WindowHost } from '@/features/window-chrome/window-host'
import type { MailxClient } from '@/lib/mailx-client'
import { Composer, type ComposerController } from './composer'

type ComposerAppProps = {
  client: MailxClient
  draftId: string
  /** The draft was created for this window (写邮件 / 回复 / 转发), so closing it unsaved deletes it. */
  fresh?: boolean
  host: WindowHost | null
}

/**
 * The composer window's page: the same canvas and title bar as the main window,
 * one island, and the host-agnostic `Composer` inside it. It also owns the close
 * protocol (sending.md section 2): a blank draft is deleted; an existing draft that
 * was not changed just closes; otherwise the user chooses to save or not. When the
 * app is quitting it saves automatically; failure cancels the quit and keeps the editor.
 */
export function ComposerApp({ client, draftId, fresh = false, host }: ComposerAppProps) {
  useTranslation('composer')
  const controller = useRef<ComposerController | null>(null)
  const [subject, setSubject] = useState('')
  const [confirmClose, setConfirmClose] = useState(false)
  const [savePrompt, setSavePrompt] = useState(false)
  const closing = useRef(false)
  const [closingNow, setClosingNow] = useState(false)
  const setClosing = useCallback((value: boolean) => {
    closing.current = value
    setClosingNow(value)
  }, [])
  const closeRequest = useRef<{ generation: number; quit: boolean } | null>(null)
  const [quitSaveFailed, setQuitSaveFailed] = useState(false)
  const removed = useRef(false)
  const [draftRemoved, setDraftRemoved] = useState(false)
  const removedAccounts = useRef(new Set<string>())
  const queryClient = useQueryClient()
  const accounts = useQuery({
    queryKey: ['composer', 'accounts'],
    queryFn: async () => (await client.listAccounts()).filter((account) => !removedAccounts.current.has(account.id)),
  })

  const title = subject.trim() || t('composer:window.newMail')
  useEffect(() => {
    document.title = title
  }, [title])

  const closeWindow = useCallback(() => {
    if (removed.current) return
    const request = closeRequest.current
    setClosing(true)
    void client.closeComposer(false, request?.generation).catch(() => { if (closeRequest.current === request) setClosing(false) })
  }, [client, setClosing])

  useEffect(() => client.onAccountRemoved((report) => {
    removedAccounts.current.add(report.accountId)
    const key = ['composer', 'accounts']
    void queryClient.cancelQueries({ queryKey: key })
    queryClient.setQueryData<Account[]>(key, (rows) => rows?.filter((account) => !removedAccounts.current.has(account.id)))
    void queryClient.invalidateQueries({ queryKey: key })
    if (removed.current || !report.draftIds.includes(draftId)) return
    removed.current = true
    setClosing(true)
    setSavePrompt(false)
    setConfirmClose(false)
    // Unmount the editor immediately. This bypasses saving, deleting and restoring
    // content; the backend already removed exactly the drafts in this report.
    setDraftRemoved(true)
    void client.closeComposer(true).catch(() => undefined)
  }), [client, draftId, queryClient, setClosing])

  const cancelClose = useCallback(() => {
    const request = closeRequest.current
    closeRequest.current = null
    setClosing(false)
    setSavePrompt(false)
    setConfirmClose(false)
    setQuitSaveFailed(false)
    if (request) void client.cancelComposerClose(request.generation).catch(() => undefined)
  }, [client, setClosing])

  // A failed or timed-out save never acknowledges a quit. A later retry is a
  // window close only; the user can request application quit again afterwards.
  const finish = useCallback(async () => {
    if (closing.current || removed.current) return
    const request = closeRequest.current
    setClosing(true)
    const current = controller.current
    if (!current) {
      closeWindow()
      return
    }
    let ok: boolean
    try {
      ok = (await current.flush()).ok
      // Cancellation can re-enable editing while a flush is still in flight.
      // Do not let its stale completion delete the now-live draft.
      if (removed.current || closeRequest.current !== request) return
      // App quit remains cancellable in another window. Preserve even an empty
      // draft until then; a delayed delete could outlive cancellation.
      if (ok && !request?.quit && current.isEmpty() && !current.isReadOnly()) await current.discard()
    } catch {
      ok = false
    }
    if (removed.current || closeRequest.current !== request) return
    if (!ok) {
      setClosing(false)
      if (request?.quit) {
        closeRequest.current = null
        setQuitSaveFailed(true)
        void client.cancelComposerClose(request.generation).catch(() => undefined)
      }
      setConfirmClose(true)
      return
    }
    closeWindow()
  }, [client, closeWindow, setClosing])

  const requestClose = useCallback(async (generation: number) => {
    if (removed.current || closing.current) return
    const request = { generation, quit: false }
    closeRequest.current = request
    setQuitSaveFailed(false)
    const current = controller.current
    if (!current || current.isReadOnly()) {
      closeWindow()
      return
    }
    if (current.isEmpty()) {
      setClosing(true)
      try {
        await current.discard()
      } catch {
        if (closeRequest.current !== request || removed.current) return
        setClosing(false)
        setConfirmClose(true)
        return
      }
      if (removed.current || closeRequest.current !== request) return
      closeWindow()
      return
    }
    if (!fresh && !current.hasChanges()) {
      await finish()
      return
    }
    setSavePrompt(true)
  }, [closeWindow, finish, fresh, setClosing])

  const saveAndClose = () => {
    setSavePrompt(false)
    setConfirmClose(false)
    void finish()
  }

  const discardAndClose = async () => {
    setSavePrompt(false)
    const request = closeRequest.current
    const current = controller.current
    if (!current || closing.current) return
    setClosing(true)
    try {
      if (fresh) await current.discard()
      else await current.restoreBaseline()
    } catch {
      if (removed.current || closeRequest.current !== request) return
      setClosing(false)
      toast.error(fresh ? t('composer:window.deleteFailed') : t('composer:window.restoreFailed'), { description: t('composer:window.keptOpen') })
      cancelClose()
      return
    }
    if (removed.current || closeRequest.current !== request) return
    closeWindow()
  }

  useEffect(() => client.onComposerCloseRequested(({ generation }) => void requestClose(generation)), [client, requestClose])
  useEffect(() => client.onAppClosing(({ generation }) => {
    if (removed.current) return
    closeRequest.current = { generation, quit: true }
    setClosing(false)
    setSavePrompt(false)
    setConfirmClose(false)
    setQuitSaveFailed(false)
    void finish()
  }), [client, finish, setClosing])
  useEffect(() => client.onAppCloseCancelled(({ generation }) => {
    if (closeRequest.current?.generation !== generation) return
    closeRequest.current = null
    setClosing(false)
    setSavePrompt(false)
    setConfirmClose(false)
    setQuitSaveFailed(false)
  }), [client, setClosing])

  return (
    <div className="app-canvas flex h-svh flex-col overflow-hidden bg-sidebar text-foreground">
      <TitleBar host={host} title={title} />
      <div className="flex min-h-0 flex-1 pr-2 pb-2 pl-2" inert={closingNow}>
        <Island>
          {!draftRemoved && <Composer
            client={client}
            draftId={draftId}
            accounts={accounts.data ?? []}
            controllerRef={controller}
            onSubjectChange={setSubject}
            onSent={closeWindow}
            onDiscarded={closeWindow}
          />}
        </Island>
      </div>
      <AlertDialog open={savePrompt} onOpenChange={(open) => { if (!open) cancelClose() }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('composer:window.saveTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t('composer:window.saveDescription')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <Button variant="ghost" className="text-destructive hover:text-destructive sm:mr-auto" onClick={() => void discardAndClose()}>
              {t('composer:window.dontSave')}</Button>
            <AlertDialogCancel>{t('composer:action.cancel')}</AlertDialogCancel>
            <Button autoFocus onClick={saveAndClose}>{t('composer:window.save')}</Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={confirmClose} onOpenChange={(open) => { if (!open) cancelClose() }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('composer:window.unsavedTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t(quitSaveFailed ? 'composer:window.quitSaveFailed' : 'composer:window.unsavedDescription')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('composer:action.cancel')}</AlertDialogCancel>
            <Button onClick={saveAndClose}>{t('composer:action.retry')}</Button>
            <Button
              variant="destructive"
              onClick={() => {
                setConfirmClose(false)
                setClosing(true)
                closeWindow()
              }}
            >
              {t('composer:window.closeAnyway')}</Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
