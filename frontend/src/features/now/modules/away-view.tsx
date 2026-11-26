import { t } from '@/i18n'
import { rowClass } from '../mail-row-card'
import { awayDuration } from './away'
import { SenderList, SenderName } from './sender-rows'
import type { NowData, NowRenderEnv } from './types'

/** One line on the time away and the new mail, then the people who wrote most. */
export function AwayView({ away, contactAccountId, onOpenContact }: {
  away: NowData['away']
  contactAccountId: string
  onOpenContact: NowRenderEnv['actions']['onOpenContact']
}) {
  return (
    <div className="flex flex-col gap-1">
      <p className="px-3 text-sm text-muted-foreground">{t('now:awayLine', { duration: awayDuration(away.ms), count: away.total })}</p>
      <SenderList>
        {away.senders.map((sender) => (
          <li key={sender.address}>
            <button type="button" onClick={() => onOpenContact({ address: sender.address, name: sender.name })} className={rowClass}>
              <SenderName sender={sender} accountId={contactAccountId} />
              <span className="shrink-0 text-xs text-muted-foreground">{t('now:senderCount', { count: sender.count })}</span>
            </button>
          </li>
        ))}
      </SenderList>
    </div>
  )
}
