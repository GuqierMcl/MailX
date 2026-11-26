import { HugeiconsIcon } from '@hugeicons/react'
import { File01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { rowClass } from '../mail-row-card'
import type { NowModule } from './types'

/** Unfinished drafts; they pile up and never clear themselves, so this is no alarm. */
export const draftsModule: NowModule = {
  id: 'drafts',
  zone: 'act',
  hideable: true,
  title: () => t('now:draftsTitle'),
  score: () => 300,
  signature: ({ draftCount }) => String(draftCount),
  hasContent: ({ draftCount }) => draftCount > 0,
  render: ({ data, actions }) => (
    <ul className="flex flex-col rounded-xl bg-muted/40 p-1">
      <li>
        <button type="button" onClick={() => actions.onSelectMailbox({ kind: 'drafts' })} className={rowClass}>
          <HugeiconsIcon icon={File01Icon} className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate">{t('now:drafts', { count: data.draftCount })}</span>
        </button>
      </li>
    </ul>
  ),
}
