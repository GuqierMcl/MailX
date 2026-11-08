import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
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

type DeleteForGoodDialogProps = {
  open: boolean
  /** Messages about to be deleted; more than one is a batch from the list selection. */
  count?: number
  /** How many of them go for good instead of to Trash. */
  permanentCount?: number
  onCancel: () => void
  onConfirm: () => void
}

/** Asks before an email (or a selection containing some) is deleted for good; see `useDeleteEmail`. */
export function DeleteForGoodDialog({ open, count = 1, permanentCount = count, onCancel, onConfirm }: DeleteForGoodDialogProps) {
  useTranslation()
  const batch = count > 1
  return (
    <AlertDialog open={open} onOpenChange={(next) => !next && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{batch ? t('mail:deleteForGoodBatchTitle', { count }) : t('mail:deleteForGoodTitle')}</AlertDialogTitle>
          <AlertDialogDescription>
            {batch && permanentCount > 0 ? t('mail:deleteForGoodBatchDescription', { count: permanentCount }) : t('mail:deleteForGoodDescription')}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('mail:cancel')}</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={onConfirm}>{batch && permanentCount < count ? t('mail:deleteBatchConfirm') : t('mail:deleteForGoodConfirm')}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
