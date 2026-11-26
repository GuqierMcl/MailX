import { HugeiconsIcon } from '@hugeicons/react'
import { Attachment01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { displayName } from '@/features/mail/format'
import { rowClass } from '../mail-row-card'
import { relativeTime } from '../relative-time'
import type { NowModule } from './types'

/** Mail with attachments from the last week. */
export const attachmentsModule: NowModule = {
  id: 'attachments',
  zone: 'flow',
  hideable: true,
  fromOverview: true,
  title: () => t('now:attachments'),
  score: () => 0,
  signature: ({ attachments }) => attachments.map(({ email }) => email.id).join('|'),
  hasContent: ({ attachments }) => attachments.length > 0,
  render: ({ data, language, minute, actions }) => (
    <ul className="flex flex-col">
      {data.attachments.map(({ email, fileNames }) => (
        <li key={email.id}>
          <button type="button" onClick={() => actions.onOpenEmail(email)} className={rowClass}>
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
  ),
}
