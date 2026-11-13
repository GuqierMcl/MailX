import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Archive02Icon, ArchiveRestoreIcon, Delete02Icon, Mail01Icon, MailOpen01Icon, StarIcon, StarOffIcon } from '@hugeicons/core-free-icons'
import type { SelectedEmail } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { IconButton } from '@/components/icon-button'
import { t } from '@/i18n'
import type { ListSelection } from './use-list-selection'
import { selectionAvailability } from './use-selection-actions'

/** What the selection header can do with the picked messages; see `useSelectionActions`. */
export type SelectionActions = {
  onSelectAll: () => void
  onFlag: (flag: 'read' | 'starred', wanted: boolean) => void
  /** Archive in inbox scopes, "move back to inbox" in the archived view. */
  archiveMode: 'archive' | 'unarchive'
  /** Whether a picked message can take the archive action (inbox mail, not a local copy). */
  canArchiveItem: (item: SelectedEmail) => boolean
  onArchive: () => void
  onDelete: () => void
  /** An action is running; the buttons wait for it. */
  busy: boolean
}

/**
 * The list header while picking messages. It takes the place of the title and
 * the filter tabs in the same row: select all / deselect all and the count on
 * the left, the actions and "Done" on the right. Actions stay visible but
 * disabled while nothing is picked.
 */
export function SelectionHeader({ selection, actions }: { selection: ListSelection; actions: SelectionActions }) {
  useTranslation()
  const { count } = selection
  const { none, changeable, anyUnread, anyUnstarred, archiveBlocked, deletable } = selectionAvailability(selection, actions)
  const archiving = actions.archiveMode === 'archive'
  const archiveLabel = archiving ? t('mail:archive') : t('mail:unarchive')
  return (
    <div role="toolbar" aria-label={t('mail:selectionActions')} className="flex min-w-0 flex-1 items-center gap-1">
      <Button variant="ghost" size="sm" className="-ml-2 shrink-0" disabled={actions.busy} onClick={selection.allSelected ? selection.deselectAll : actions.onSelectAll}>
        {selection.allSelected ? t('mail:deselectAll') : t('mail:selectAll')}
      </Button>
      <span aria-live="polite" className="min-w-0 truncate text-sm text-muted-foreground">
        {count === 0 ? t('mail:selectMessages') : t('mail:selectedCount', { count })}
      </span>
      <div className="ml-auto flex shrink-0 items-center gap-0.5">
        <IconButton
          label={anyUnread ? t('mail:markRead') : t('mail:markUnread')}
          disabled={none || changeable === 0}
          onClick={() => actions.onFlag('read', anyUnread)}
        >
          <HugeiconsIcon icon={anyUnread ? MailOpen01Icon : Mail01Icon} />
        </IconButton>
        <IconButton
          label={anyUnstarred ? t('mail:star') : t('mail:unstar')}
          disabled={none || changeable === 0}
          onClick={() => actions.onFlag('starred', anyUnstarred)}
        >
          <HugeiconsIcon icon={anyUnstarred ? StarIcon : StarOffIcon} />
        </IconButton>
        <IconButton
          label={archiveLabel}
          tip={archiveBlocked ? (archiving ? t('mail:archiveInboxOnly') : t('mail:unarchiveArchivedOnly')) : undefined}
          disabled={none || archiveBlocked}
          onClick={actions.onArchive}
        >
          <HugeiconsIcon icon={archiving ? Archive02Icon : ArchiveRestoreIcon} />
        </IconButton>
        <IconButton
          label={t('mail:delete')}
          tip={count > 0 && deletable === 0 ? t('mail:deleteLocalOnlyBatch') : undefined}
          disabled={none || deletable === 0}
          className="text-destructive hover:text-destructive"
          onClick={actions.onDelete}
        >
          <HugeiconsIcon icon={Delete02Icon} />
        </IconButton>
        <Button variant="ghost" size="sm" className="ml-1" onClick={selection.exit}>{t('mail:doneSelecting')}</Button>
      </div>
    </div>
  )
}
