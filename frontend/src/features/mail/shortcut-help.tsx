import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { ShortcutList } from './shortcut-list'

/** The `?` overlay: every shortcut, and the way to the settings page that can turn the single keys off. */
export function ShortcutHelpDialog({ open, onOpenChange, onOpenSettings }: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onOpenSettings: () => void
}) {
  useTranslation()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="flex max-h-[min(640px,calc(100dvh-4rem))] flex-col sm:max-w-md"
        onKeyDown={(event) => {
          // `?` closes it again; Esc is the dialog's own.
          if (event.key === '?' && !event.metaKey && !event.ctrlKey && !event.altKey) {
            event.preventDefault()
            onOpenChange(false)
          }
        }}
      >
        <DialogHeader>
          <DialogTitle>{t('mail:shortcutsTitle')}</DialogTitle>
          <DialogDescription>{t('mail:shortcutsDescription')}</DialogDescription>
        </DialogHeader>
        <ShortcutList className="min-h-0 overflow-y-auto" />
        <DialogFooter>
          <Button variant="outline" size="sm" onClick={onOpenSettings}>{t('mail:shortcutsOpenSettings')}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
