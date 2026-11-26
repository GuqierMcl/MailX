import { useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, Attachment01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import type { RecentAttachment } from '@/generated/accounts'
import { cn } from '@/lib/utils'
import { displayName } from '@/features/mail/format'
import { rowClass } from '../mail-row-card'
import { relativeTime } from '../relative-time'
import type { NowRenderEnv } from './types'

export const ATTACHMENTS_OPEN_KEY = 'mailx.now.attachmentsOpen'
/** The most attachment mails the overview lists; a full list means there may be more. */
const LISTED = 5

function readOpen(): boolean {
  try {
    return globalThis.localStorage?.getItem(ATTACHMENTS_OPEN_KEY) === '1'
  } catch {
    return false
  }
}

function writeOpen(open: boolean) {
  try {
    globalThis.localStorage?.setItem(ATTACHMENTS_OPEN_KEY, open ? '1' : '0')
  } catch {
    // The choice is just forgotten.
  }
}

/** One muted line with a chevron; the list of mail with attachments opens below it and the choice is remembered. */
export function AttachmentsView({ attachments, language, minute, onOpenEmail }: {
  attachments: RecentAttachment[]
  language: string
  minute: number
  onOpenEmail: NowRenderEnv['actions']['onOpenEmail']
}) {
  const [open, setOpen] = useState(readOpen)
  const toggle = () => {
    writeOpen(!open)
    setOpen(!open)
  }
  const count = attachments.length
  return (
    <div className="flex flex-col gap-1">
      <button
        type="button"
        aria-expanded={open}
        onClick={toggle}
        className="flex items-center gap-1 self-start rounded-md px-3 py-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span>{t(count >= LISTED ? 'now:attachmentsLineAtLeast' : 'now:attachmentsLine', { count })}</span>
        <HugeiconsIcon icon={ArrowDown01Icon} aria-hidden strokeWidth={2} className={cn('size-3.5 transition-transform duration-150 motion-reduce:transition-none', open && 'rotate-180')} />
      </button>
      {open && (
        <ul className="flex flex-col">
          {attachments.map(({ email, fileNames }) => (
            <li key={email.id}>
              <button type="button" onClick={() => onOpenEmail(email)} className={rowClass}>
                <HugeiconsIcon icon={Attachment01Icon} className="size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate">
                    {fileNames[0] ?? t('now:attachment')}
                    {fileNames.length > 1 && <span className="text-muted-foreground"> {t('now:moreFiles', { count: fileNames.length - 1 })}</span>}
                  </span>
                  <span className="block truncate text-xs text-muted-foreground">{displayName(email.from)}</span>
                </span>
                <span className="shrink-0 text-xs text-muted-foreground">{relativeTime(email.receivedAt, language, minute)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
