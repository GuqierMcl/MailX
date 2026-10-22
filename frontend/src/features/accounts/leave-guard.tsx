import { useState } from 'react'
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

/**
 * Confirms before leaving a form with unsaved input. `leave(action)` runs the
 * action at once when nothing is unsaved, asks first otherwise, and does
 * nothing while a save is in flight. Render `confirm` once next to the form.
 */
export function useLeaveGuard() {
  const [dirty, setDirty] = useState(false)
  const [busy, setBusy] = useState(false)
  const [pending, setPending] = useState<(() => void) | null>(null)

  const leave = (action: () => void, unsaved = dirty) => {
    if (busy) return
    const run = () => {
      setDirty(false)
      action()
    }
    if (unsaved) setPending(() => run)
    else run()
  }

  const confirm = (
    <AlertDialog open={pending !== null} onOpenChange={(open) => !open && setPending(null)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>放弃未保存的修改？</AlertDialogTitle>
          <AlertDialogDescription>当前表单的修改和已输入的密码将被清除。</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>继续编辑</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => {
              pending?.()
              setPending(null)
            }}
          >
            放弃修改
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )

  return { dirty, busy, setDirty, setBusy, leave, confirm }
}
