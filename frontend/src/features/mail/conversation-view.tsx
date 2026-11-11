import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Attachment01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import type { ConversationPage, EmailSummary } from '@/generated/accounts'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { cn } from '@/lib/utils'
import { displayName, formatListTime, threadSubject } from './format'
import { SenderAvatar } from './sender-avatar'

export type ThreadMode = 'thread' | 'single'

/** How many collapsed rows show on each side of the expanded message before the rest fold away. */
export const VISIBLE_SIDE_ROWS = 3

type FrameProps = {
  page: ConversationPage
  /** The message shown in full; every other message is a collapsed row. */
  expandedId: string
  /** The owning account's address, so own messages read "Me". */
  selfAddress: string | null
  mode: ThreadMode
  onModeChange: (mode: ThreadMode) => void
  onExpand: (message: EmailSummary) => void
  /** The reader's own content for the expanded message (header, body, attachments). */
  children: ReactNode
}

/**
 * Wraps the reader's content for the expanded message with the rest of its
 * conversation. The content itself is untouched, so the message body still
 * renders in its own sandboxed document.
 */
export function ConversationFrame({ page, expandedId, selfAddress, mode, onModeChange, onExpand, children }: FrameProps) {
  useTranslation()
  const [showAllEarlier, setShowAllEarlier] = useState(false)
  const [showAllNewer, setShowAllNewer] = useState(false)
  const { messages } = page
  const position = messages.findIndex((message) => message.id === expandedId)
  const earlier = messages.slice(0, Math.max(position, 0))
  const newer = messages.slice(position + 1)
  const single = mode === 'single'
  const hiddenEarlier = showAllEarlier ? 0 : Math.max(0, earlier.length - VISIBLE_SIDE_ROWS)
  const hiddenNewer = showAllNewer ? 0 : Math.max(0, newer.length - VISIBLE_SIDE_ROWS)
  const row = (message: EmailSummary) => (
    <CollapsedRow key={message.id} message={message} selfAddress={selfAddress} onExpand={onExpand} />
  )

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', !single && 'overflow-y-auto')}>
      <div className="flex shrink-0 flex-col gap-0.5 px-4 pb-2">
        <div className="flex items-center gap-3">
          {!single && <p className="min-w-0 truncate text-sm font-medium">{threadSubject(messages[0]?.subject ?? '') || t('mail:noSubject')}</p>}
          <span className="shrink-0 text-xs text-muted-foreground">{t('mail:threadCount', { count: messages.length })}</span>
          <Tabs className="ml-auto shrink-0" value={mode} onValueChange={(value) => onModeChange(value as ThreadMode)}>
            <TabsList aria-label={t('mail:threadViewLabel')}>
              <TabsTrigger value="single">{t('mail:threadViewSingle')}</TabsTrigger>
              <TabsTrigger value="thread">{t('mail:threadViewConversation')}</TabsTrigger>
            </TabsList>
          </Tabs>
        </div>
        {!single && page.possiblyIncomplete && <p className="text-xs text-muted-foreground">{t('mail:threadIncomplete')}</p>}
      </div>
      {single ? (
        <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      ) : (
        <>
          <div role="group" aria-label={t('mail:threadMessages')} className="contents">
            {hiddenEarlier > 0 && (
              <FoldedRow label={t('mail:threadEarlier', { count: hiddenEarlier })} onClick={() => setShowAllEarlier(true)} />
            )}
            {earlier.slice(hiddenEarlier > 0 ? hiddenEarlier : 0).map(row)}
          </div>
          <div className="flex min-h-80 flex-1 flex-col">{children}</div>
          <div role="group" aria-label={t('mail:threadMessages')} className="contents">
            {newer.slice(0, newer.length - hiddenNewer).map(row)}
            {hiddenNewer > 0 && (
              <FoldedRow label={t('mail:threadNewer', { count: hiddenNewer })} onClick={() => setShowAllNewer(true)} />
            )}
          </div>
        </>
      )}
    </div>
  )
}

const ROW_CLASS = 'flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left text-sm transition-colors hover:bg-foreground/5 focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none'

function FoldedRow({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <div className="shrink-0 px-2">
      <button type="button" className={cn(ROW_CLASS, 'text-xs text-muted-foreground')} onClick={onClick}>
        <span className="w-8 shrink-0" />
        {label}
      </button>
    </div>
  )
}

function CollapsedRow({ message, selfAddress, onExpand }: { message: EmailSummary; selfAddress: string | null; onExpand: (message: EmailSummary) => void }) {
  useTranslation()
  const mine = selfAddress !== null && message.from.address.toLowerCase() === selfAddress.toLowerCase()
  const name = mine ? t('mail:threadMe') : displayName(message.from)
  const time = formatListTime(message.receivedAt)
  // Thread rows are often outside the visible list (e.g. in Sent), so nothing asks for
  // their preview: show it once known, never a "loading" line that would not finish.
  const known = message.bodyState === 'ready' || message.previewState === 'ready'
  const preview = (known ? message.preview : '') || message.subject || t('mail:noSubject')
  return (
    <div className="shrink-0 px-2">
      <button
        type="button"
        className={ROW_CLASS}
        aria-label={t(message.isRead ? 'mail:threadRowLabel' : 'mail:threadRowLabelUnread', { name, time, preview })}
        onClick={() => onExpand(message)}
      >
        <SenderAvatar sender={message.from} accountId={message.accountId} size="default" />
        <span className={cn('w-28 shrink-0 truncate', !message.isRead && 'font-medium')}>{name}</span>
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{preview}</span>
        {message.hasAttachment && <HugeiconsIcon icon={Attachment01Icon} aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />}
        <time dateTime={message.receivedAt} className="shrink-0 text-xs text-muted-foreground">{time}</time>
        <span aria-hidden className={cn('size-2 shrink-0 rounded-full', !message.isRead && 'bg-blue-500 dark:bg-blue-400')} />
      </button>
    </div>
  )
}
