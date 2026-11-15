import { useState } from 'react'
import type { EmailListQuery, Folder, SelectedEmail } from '@/generated/accounts'
import { toast } from '@/components/ui/toast'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import type { MailxClient } from '@/lib/mailx-client'
import type { SelectionActions } from './selection-header'
import { canArchive } from './use-archive-email'
import type { BatchResult, BatchSettle } from './use-delete-email'
import type { ListSelection } from './use-list-selection'

type Options = {
  client: MailxClient
  selection: ListSelection
  /** The query the list shows; "select all" asks the host for the same messages. */
  listQuery: EmailListQuery
  folders: Folder[]
  /** The local archived view, where archiving turns into "move back to inbox". */
  archivedView: boolean
  deleteMany: (items: SelectedEmail[], settle: BatchSettle) => void
  archiveMany: (items: SelectedEmail[], archive: boolean, applicable: (item: SelectedEmail) => boolean, settle: BatchSettle) => Promise<void>
  flagMany: (items: SelectedEmail[], flag: 'read' | 'starred', wanted: boolean, settle: BatchSettle) => Promise<void>
}

/**
 * Connects the selection header to the batch variants of the delete, archive
 * and flag hooks. After an action the finished messages leave the selection
 * and the ones that could not be changed stay picked; the mode stays on.
 */
export function useSelectionActions({ client, selection, listQuery, folders, archivedView, deleteMany, archiveMany, flagMany }: Options): SelectionActions {
  const [busy, setBusy] = useState(false)
  const canArchiveItem = (item: SelectedEmail) => archivedView
    ? item.archived && !item.localOnly
    : !item.archived && canArchive(item, folders)

  /** Starts an action unless one is running; `leave` says what drops out of the selection afterwards. */
  const run = (action: (settle: BatchSettle) => void, leave: (result: BatchResult) => void) => {
    if (busy || selection.items.length === 0) return
    setBusy(true)
    action((result) => {
      leave(result)
      setBusy(false)
    })
  }
  // Deleted and archived messages leave the view, so they leave the selection.
  const leaveDone = (result: BatchResult) => selection.drop(result.done)

  return {
    busy,
    archiveMode: archivedView ? 'unarchive' : 'archive',
    canArchiveItem,
    onSelectAll: () => {
      void selection.selectAll(() => client.selectEmails({ ...listQuery, limit: null, cursor: null })).catch((error: unknown) => {
        toast.error(t('mail:selectAllFailed'), { description: errorText(error) })
      })
    },
    onFlag: (flag, wanted) => run(
      (settle) => void flagMany(selection.items, flag, wanted, settle),
      // The rows stay listed; only after a partial failure does the selection
      // narrow to the messages that still need another try.
      (result) => { if (result.failed.length > 0) selection.keep(result.failed) },
    ),
    onArchive: () => run((settle) => void archiveMany(selection.items, !archivedView, canArchiveItem, settle), leaveDone),
    onDelete: () => run((settle) => deleteMany(selection.items, settle), leaveDone),
  }
}

/**
 * What the selection header (and its keyboard shortcuts) can do right now with
 * the picked messages. Mixed selections go the way that changes something: any
 * unread → mark read. With nothing picked the flags rest on "mark as read" and
 * "add star".
 */
export function selectionAvailability(selection: Pick<ListSelection, 'items' | 'count'>, actions: Pick<SelectionActions, 'busy' | 'canArchiveItem'>) {
  const { items, count } = selection
  const none = count === 0 || actions.busy
  const changeable = items.filter((item) => !item.localOnly)
  const archivable = items.filter(actions.canArchiveItem).length
  return {
    none,
    changeable: changeable.length,
    anyUnread: changeable.length === 0 || changeable.some((item) => !item.isRead),
    anyUnstarred: changeable.length === 0 || changeable.some((item) => !item.isStarred),
    archivable,
    archiveBlocked: count > 0 && archivable === 0,
    deletable: changeable.length,
  }
}
