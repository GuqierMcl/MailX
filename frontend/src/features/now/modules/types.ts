import type { ReactNode } from 'react'
import type { IconSvgElement } from '@hugeicons/react'
import type { Account, EmailSummary, Folder, OverviewActivity, OverviewCode, OverviewSender, RecentAttachment } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import type { VerificationCodes, ReadyVerificationCode } from '@/features/mail/use-verification-codes'
import type { Mailbox } from '@/features/mail/mailbox'
import type { EntranceMode } from '../motion'

/** The modules the page knows (now-page.md 4.3); the registry lists them in this default order. */
export type NowModuleId = 'attention' | 'codes' | 'drafts' | 'away' | 'inbox' | 'cleanup' | 'activity' | 'attachments'

/**
 * A module's zone sets the base of its score (act 300, flow 200, review 100); the automatic
 * arrangement sorts by score alone, so a module can rise one zone but not further.
 */
export type NowZone = 'act' | 'flow' | 'review'

export type AttentionItem = { id: string; icon: IconSvgElement; label: string; tone?: 'alert'; onClick: () => void; avatar?: Account }

/** A code the shared recognizer confirmed, whether the overview found it or only suspected it. */
export type ShownCode = Pick<OverviewCode, 'emailId' | 'accountId' | 'from' | 'receivedAt'> & { ready: ReadyVerificationCode }

export type AccountInboxUnread = { account: Account; folder: Folder; unread: number }

/** What the page knows right now, shaped for the modules. */
export type NowData = {
  /** Stopped accounts and unfinished sends that went wrong; never drafts. */
  attention: AttentionItem[]
  draftCount: number
  codes: ShownCode[]
  inbox: { newMail: number; unread: number; starredUnread: number; rows: EmailSummary[]; accountUnread?: AccountInboxUnread[] }
  attachments: RecentAttachment[]
  /** Absent while the overview is missing. */
  activity: OverviewActivity | undefined
  /** The time away and the new mail it brought; `senders` are the ones who wrote most. */
  away: { sinceTime: number; ms: number; total: number; senders: OverviewSender[] }
  /** Senders of at least three unread inbox messages, most first. */
  unreadSenders: OverviewSender[]
}

/** What decides the automatic arrangement besides a module's own data. */
export type NowContext = {
  now: Date
  /** Milliseconds since the user last looked at the page. */
  awayMs: number
  /** The day's first visit. */
  firstToday: boolean
  data: NowData
}

/** What a module needs to draw itself and to lead somewhere. */
export type NowRenderEnv = {
  data: NowData
  language: string
  /** The page's minute clock, for relative times. */
  minute: number
  mode: EntranceMode
  /** When the user last looked, as a timestamp. */
  sinceTime: number
  verification: VerificationCodes
  /** Any account; avatars are looked up by address. */
  contactAccountId: string
  client: MailxClient
  folders: Folder[]
  mailActions?: {
    markRead: (email: EmailSummary) => void
    toggleStar: (email: EmailSummary) => void
    archive: (email: EmailSummary) => void
    busy: ReadonlySet<string>
  }
  actions: {
    onSelectMailbox: (mailbox: Mailbox) => void
    onOpenUnread: () => void
    /** One person's mail across folders and accounts. */
    onOpenContact: (contact: { address: string; name: string | null }) => void
    onOpenAccount: (accountId: string) => void
    onOpenEmail: (email: { id: string; accountId: string }) => void
  }
}

export type NowModule = {
  id: NowModuleId
  zone: NowZone
  /** False for what the user must not miss: it ignores hiding and snoozing. */
  hideable: boolean
  /** Whether the data comes from the overview query, so a failed query is reported where the module would be. */
  fromOverview?: boolean
  title: () => string
  /** Draws no title row of its own: the module's content leads, with the "more" button at its end. */
  compact?: boolean
  /** Higher comes first; equal scores keep the registry order (now-page.md 4.3 has the table). */
  score: (context: NowContext) => number
  /** Changes when the content does; a module hidden for today comes back when it changes. */
  signature: (data: NowData) => string
  /** A module without content is not shown, however it is arranged. */
  hasContent: (data: NowData) => boolean
  /** The module's body; the page draws the title row. */
  render: (env: NowRenderEnv) => ReactNode
}
