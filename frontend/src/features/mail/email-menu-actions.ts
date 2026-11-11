import type { IconSvgElement } from '@hugeicons/react'
import {
  Archive02Icon,
  ArchiveRestoreIcon,
  Delete02Icon,
  Forward01Icon,
  Mail01Icon,
  MailOpen01Icon,
  MailReply01Icon,
  MailReplyAll01Icon,
  StarIcon,
  UserMultiple02Icon,
  UserSearch01Icon,
  ViewIcon,
} from '@hugeicons/core-free-icons'
import type { EmailSummary } from '@/generated/accounts'

export type RespondKind = 'reply' | 'replyAll' | 'forward'

/** The message actions a list offers; a missing callback hides its items. */
export type EmailMenuCallbacks = {
  onOpen?: (email: EmailSummary) => void
  onRespond?: (email: EmailSummary, kind: RespondKind) => void
  onFlag?: (email: EmailSummary, flag: 'read' | 'starred', wanted: boolean) => void
  onArchive?: (email: EmailSummary, archive: boolean) => void
  onDelete?: (email: EmailSummary) => void
  onRemoveLocal?: (email: EmailSummary) => void
  /** Whether local archive is available for this message's current folder. */
  canArchive?: (email: EmailSummary) => boolean
  /** Opens the contact view for the message's sender. */
  onViewContact?: (email: EmailSummary) => void
  /** Turns grouping of the sender's mail on or off. */
  onAggregate?: (email: EmailSummary, aggregate: boolean) => void
  /** Whether the sender's mail is already grouped. */
  isAggregated?: (email: EmailSummary) => boolean
  /** The sender whose contact view is on screen, if any; "view" is not offered for it again. */
  viewingContact?: string | null
}

export type EmailMenuAction = {
  kind: 'open' | RespondKind | 'viewContact' | 'aggregate' | 'unaggregate' | 'read' | 'unread' | 'star' | 'unstar' | 'archive' | 'unarchive' | 'delete' | 'removeLocal'
  label:
    | 'mail:openMessage' | 'mail:reply' | 'mail:replyAll' | 'mail:forward'
    | 'mail:markRead' | 'mail:markUnread' | 'mail:star' | 'mail:unstar'
    | 'mail:archive' | 'mail:unarchive' | 'mail:delete' | 'mail:removeLocalCopy'
    | 'mail:viewContactMail' | 'mail:aggregateContact' | 'mail:unaggregateContact'
  icon: IconSvgElement
  destructive?: boolean
  run: () => void
}

/**
 * The menu for one message, as groups of actions. Rules match the row's quick
 * actions: a local-only sent copy has no server message, so it offers no read,
 * star, archive, delete or contact items, only removal from this device. Empty groups are dropped.
 */
export function emailMenuActions(email: EmailSummary, available: EmailMenuCallbacks): EmailMenuAction[][] {
  const { onOpen, onRespond, onFlag, onArchive, onDelete, onRemoveLocal, canArchive, onViewContact, onAggregate, isAggregated, viewingContact } = available
  const localOnly = email.localOnly
  const open: EmailMenuAction[] = onOpen ? [{ kind: 'open', label: 'mail:openMessage', icon: ViewIcon, run: () => onOpen(email) }] : []
  const respond: EmailMenuAction[] = onRespond
    ? ([
        ['reply', 'mail:reply', MailReply01Icon],
        ['replyAll', 'mail:replyAll', MailReplyAll01Icon],
        ['forward', 'mail:forward', Forward01Icon],
      ] as const).map(([kind, label, icon]) => ({ kind, label, icon, run: () => onRespond(email, kind) }))
    : []
  // Rows of a local-only copy or without a sender address have no contact to speak of.
  const sender = email.from.address.trim().toLowerCase()
  const hasContact = !localOnly && sender !== ''
  const contact: EmailMenuAction[] = []
  if (hasContact && onViewContact && sender !== viewingContact?.trim().toLowerCase()) {
    contact.push({ kind: 'viewContact', label: 'mail:viewContactMail', icon: UserSearch01Icon, run: () => onViewContact(email) })
  }
  if (hasContact && onAggregate) {
    contact.push(isAggregated?.(email)
      ? { kind: 'unaggregate', label: 'mail:unaggregateContact', icon: UserMultiple02Icon, run: () => onAggregate(email, false) }
      : { kind: 'aggregate', label: 'mail:aggregateContact', icon: UserMultiple02Icon, run: () => onAggregate(email, true) })
  }
  const flags: EmailMenuAction[] = onFlag && !localOnly
    ? [
        email.isRead
          ? { kind: 'unread', label: 'mail:markUnread', icon: Mail01Icon, run: () => onFlag(email, 'read', false) }
          : { kind: 'read', label: 'mail:markRead', icon: MailOpen01Icon, run: () => onFlag(email, 'read', true) },
        email.isStarred
          ? { kind: 'unstar', label: 'mail:unstar', icon: StarIcon, run: () => onFlag(email, 'starred', false) }
          : { kind: 'star', label: 'mail:star', icon: StarIcon, run: () => onFlag(email, 'starred', true) },
      ]
    : []
  const archive: EmailMenuAction[] = onArchive && !localOnly && canArchive?.(email)
    ? [email.archived
        ? { kind: 'unarchive', label: 'mail:unarchive', icon: ArchiveRestoreIcon, run: () => onArchive(email, false) }
        : { kind: 'archive', label: 'mail:archive', icon: Archive02Icon, run: () => onArchive(email, true) }]
    : []
  const remove: EmailMenuAction[] = localOnly
    ? onRemoveLocal ? [{ kind: 'removeLocal', label: 'mail:removeLocalCopy', icon: Delete02Icon, destructive: true, run: () => onRemoveLocal(email) }] : []
    : onDelete ? [{ kind: 'delete', label: 'mail:delete', icon: Delete02Icon, destructive: true, run: () => onDelete(email) }] : []
  return [open, respond, contact, flags, archive, remove].filter((group) => group.length > 0)
}
