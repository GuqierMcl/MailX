import { useCallback, useEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
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

export const NEW_MAIL_TITLE = '新邮件'

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
  const controller = useRef<ComposerController | null>(null)
  const [subject, setSubject] = useState('')
  const [confirmClose, setConfirmClose] = useState(false)
  const [savePrompt, setSavePrompt] = useState(false)
  const closing = useRef(false)
  const accounts = useQuery({ queryKey: ['composer', 'accounts'], queryFn: () => client.listAccounts() })

  const title = subject.trim() || NEW_MAIL_TITLE
  useEffect(() => {
    document.title = title
  }, [title])

  const closeWindow = useCallback(() => {
    void client.closeComposer().catch(() => undefined)
  }, [client])

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
    if (!ok && askOnFailure) {
      closing.current = false
      setConfirmClose(true)
      return
    }
    if (ok && current.isEmpty() && !current.isReadOnly()) await current.discard().catch(() => undefined)
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
      closing.current = false
      toast.error(fresh ? '没能删除这份草稿' : '没能还原这份草稿', { description: '窗口保持打开，请重试。' })
      return
    }
    closeWindow()
  }

  useEffect(() => client.onComposerCloseRequested(() => void requestClose()), [client, requestClose])
  useEffect(() => client.onAppClosing(() => void finish(false)), [client, finish])

  return (
    <div className="app-canvas flex h-svh flex-col overflow-hidden bg-sidebar text-foreground">
      <TitleBar host={host} title={title} />
      <div className="flex min-h-0 flex-1 pr-2 pb-2 pl-2">
        <Island>
          <Composer
            client={client}
            draftId={draftId}
            accounts={accounts.data ?? []}
            controllerRef={controller}
            onSubjectChange={setSubject}
            onSent={closeWindow}
            onDiscarded={closeWindow}
          />
        </Island>
      </div>
      <AlertDialog open={savePrompt} onOpenChange={setSavePrompt}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>要保存这封草稿吗？</AlertDialogTitle>
            <AlertDialogDescription>保存后可以在“草稿”中继续编辑。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <Button variant="ghost" className="text-destructive hover:text-destructive sm:mr-auto" onClick={() => void discardAndClose()}>
              不保存
            </Button>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction autoFocus onClick={saveAndClose}>保存草稿</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={confirmClose} onOpenChange={setConfirmClose}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>草稿尚未保存</AlertDialogTitle>
            <AlertDialogDescription>最近的修改没有写入本机。现在关闭会丢失这些修改。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmClose(false)
                closing.current = true
                closeWindow()
              }}
            >
              仍要关闭（丢失未保存的修改）
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
