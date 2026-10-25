import { t } from '@/i18n'
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
          <AlertDialogTitle>{t('accounts:discardChangesQuestion')}</AlertDialogTitle>
          <AlertDialogDescription>{t('accounts:discardChangesHint')}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('accounts:continueEditing')}</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => {
              pending?.()
              setPending(null)
            }}
          >

            {t('accounts:discardChanges')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )

  return { dirty, busy, setDirty, setBusy, leave, confirm }
}
