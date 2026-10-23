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
import { Island } from '@/features/shell/island'
import { TitleBar } from '@/features/window-chrome/title-bar'
import type { WindowHost } from '@/features/window-chrome/window-host'
import type { MailxClient } from '@/lib/mailx-client'
import { Composer, type ComposerController } from './composer'

export const NEW_MAIL_TITLE = '新邮件'

type ComposerAppProps = {
  client: MailxClient
  draftId: string
  host: WindowHost | null
}

/**
 * The composer window's page: the same canvas and title bar as the main window,
 * one island, and the host-agnostic `Composer` inside it. It also owns the close
 * protocol (sending.md section 2): on a close request or the app quitting, write
 * the draft, delete it when it is completely empty, then ask the host to close.
 */
export function ComposerApp({ client, draftId, host }: ComposerAppProps) {
  const controller = useRef<ComposerController | null>(null)
  const [subject, setSubject] = useState('')
  const [confirmClose, setConfirmClose] = useState(false)
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

  useEffect(() => client.onComposerCloseRequested(() => void finish(true)), [client, finish])
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
