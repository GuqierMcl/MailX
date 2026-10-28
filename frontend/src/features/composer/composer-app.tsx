import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { Account } from '@/generated/accounts'
import {
  AlertDialog,
  AlertDialogAction,
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
 * app is quitting nobody is asked: the draft is written and closed.
 */
export function ComposerApp({ client, draftId, fresh = false, host }: ComposerAppProps) {
  useTranslation('composer')
  const controller = useRef<ComposerController | null>(null)
  const [subject, setSubject] = useState('')
  const [confirmClose, setConfirmClose] = useState(false)
  const [savePrompt, setSavePrompt] = useState(false)
  const closing = useRef(false)
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
    void client.closeComposer().catch(() => undefined)
  }, [client])

  useEffect(() => client.onAccountRemoved((report) => {
    removedAccounts.current.add(report.accountId)
    const key = ['composer', 'accounts']
    void queryClient.cancelQueries({ queryKey: key })
    queryClient.setQueryData<Account[]>(key, (rows) => rows?.filter((account) => !removedAccounts.current.has(account.id)))
    void queryClient.invalidateQueries({ queryKey: key })
    if (removed.current || !report.draftIds.includes(draftId)) return
    removed.current = true
    closing.current = true
    setSavePrompt(false)
    setConfirmClose(false)
    // Unmount the editor immediately. This bypasses saving, deleting and restoring
    // content; the backend already removed exactly the drafts in this report.
    setDraftRemoved(true)
    void client.closeComposer(true).catch(() => undefined)
  }), [client, draftId, queryClient])

  // Writes the draft, drops it when empty, then closes. `askOnFailure` is for the
  // system's close request: if the write failed, the user decides. When the whole
  // app is quitting the host closes us anyway, so nobody is asked.
  const finish = useCallback(async (askOnFailure: boolean) => {
    if (closing.current) return
    closing.current = true
    const current = controller.current
    if (!current) {
      closeWindow()
      return
    }
    const { ok } = await current.flush()
    if (removed.current) return
    if (!ok && askOnFailure) {
      closing.current = false
      setConfirmClose(true)
      return
    }
    if (ok && current.isEmpty() && !current.isReadOnly()) await current.discard().catch(() => undefined)
    if (removed.current) return
    closeWindow()
  }, [closeWindow])

  // The window's own close request (button, Cmd+W, system).
  const requestClose = useCallback(async () => {
    if (closing.current) return
    const current = controller.current
    if (!current || current.isReadOnly()) {
      closeWindow()
      return
    }
    if (current.isEmpty()) {
      closing.current = true
      await current.discard().catch(() => undefined)
      if (removed.current) return
      closeWindow()
      return
    }
    if (!fresh && !current.hasChanges()) {
      await finish(true)
      return
    }
    setSavePrompt(true)
  }, [closeWindow, finish, fresh])

  const saveAndClose = () => {
    setSavePrompt(false)
    void finish(true)
  }

  const discardAndClose = async () => {
    setSavePrompt(false)
    const current = controller.current
    if (!current || closing.current) return
    closing.current = true
    try {
      // A new draft goes away; an existing one only loses this session's edits.
      if (fresh) await current.discard()
      else await current.restoreBaseline()
    } catch {
      if (removed.current) return
      closing.current = false
      toast.error(fresh ? t('composer:window.deleteFailed') : t('composer:window.restoreFailed'), { description: t('composer:window.keptOpen') })
      return
    }
    if (removed.current) return
    closeWindow()
  }

  useEffect(() => client.onComposerCloseRequested(() => void requestClose()), [client, requestClose])
  useEffect(() => client.onAppClosing(() => void finish(false)), [client, finish])

  return (
    <div className="app-canvas flex h-svh flex-col overflow-hidden bg-sidebar text-foreground">
      <TitleBar host={host} title={title} />
      <div className="flex min-h-0 flex-1 pr-2 pb-2 pl-2">
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
      <AlertDialog open={savePrompt} onOpenChange={setSavePrompt}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('composer:window.saveTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t('composer:window.saveDescription')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <Button variant="ghost" className="text-destructive hover:text-destructive sm:mr-auto" onClick={() => void discardAndClose()}>
              {t('composer:window.dontSave')}</Button>
            <AlertDialogCancel>{t('composer:action.cancel')}</AlertDialogCancel>
            <AlertDialogAction autoFocus onClick={saveAndClose}>{t('composer:window.save')}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={confirmClose} onOpenChange={setConfirmClose}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('composer:window.unsavedTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t('composer:window.unsavedDescription')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('composer:action.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmClose(false)
                closing.current = true
                closeWindow()
              }}
            >
              {t('composer:window.closeAnyway')}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
