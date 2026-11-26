import type { ReactNode } from 'react'
import type { IconSvgElement } from '@hugeicons/react'
import type { Account, EmailSummary, OverviewActivity, OverviewContact, OverviewCode, RecentAttachment } from '@/generated/accounts'
import type { VerificationCodes, ReadyVerificationCode } from '@/features/mail/use-verification-codes'
import type { Mailbox } from '@/features/mail/mailbox'
import type { EntranceMode } from '../motion'

/** The modules the page knows (now-page.md 4.3); the registry lists them in this default order. */
export type NowModuleId = 'attention' | 'codes' | 'drafts' | 'inbox' | 'attachments' | 'activity'

/** Zones keep their order in the automatic arrangement: act, then flow, then review. */
export type NowZone = 'act' | 'flow' | 'review'

export type AttentionItem = { id: string; icon: IconSvgElement; label: string; tone?: 'alert'; onClick: () => void; avatar?: Account }

/** A code the shared recognizer confirmed, whether the overview found it or only suspected it. */
export type ShownCode = Pick<OverviewCode, 'emailId' | 'accountId' | 'from' | 'receivedAt'> & { ready: ReadyVerificationCode }

/** What the page knows right now, shaped for the modules. */
export type NowData = {
  /** Stopped accounts and unfinished sends that went wrong; never drafts. */
  attention: AttentionItem[]
  draftCount: number
  codes: ShownCode[]
  inbox: { newMail: number; unread: number; starredUnread: number; rows: EmailSummary[] }
  attachments: RecentAttachment[]
  /** Absent while the overview is missing. */
  activity: OverviewActivity | undefined
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
  actions: {
    onSelectMailbox: (mailbox: Mailbox) => void
    onOpenUnread: () => void
    onOpenContact: (contact: OverviewContact) => void
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
  /** Higher comes first within the zone; equal scores keep the registry order. */
  score: (context: NowContext) => number
  /** Changes when the content does; a module hidden for today comes back when it changes. */
  signature: (data: NowData) => string
  /** A module without content is not shown, however it is arranged. */
  hasContent: (data: NowData) => boolean
  /** The module's body; the page draws the title row. */
  render: (env: NowRenderEnv) => ReactNode
}
