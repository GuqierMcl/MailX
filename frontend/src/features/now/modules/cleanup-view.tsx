import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import type { OverviewSender } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { useMarkSenderRead } from '../use-mark-sender-read'
import { senderLabel } from './sender-label'
import { SenderList } from './sender-rows'
import type { NowRenderEnv } from './types'

/** Rows of senders with piled-up unread mail, each with "mark all as read" and "view". */
export function CleanupView({ client, senders, contactAccountId, onOpenContact }: {
  client: MailxClient
  senders: OverviewSender[]
  contactAccountId: string
  onOpenContact: NowRenderEnv['actions']['onOpenContact']
}) {
  const { markRead, busy } = useMarkSenderRead(client)
  return (
    <SenderList>
      {senders.map((sender) => {
        const label = senderLabel(sender)
        return (
          <li key={sender.address} className="flex items-center gap-3 rounded-lg px-3 py-1.5 text-sm hover:bg-foreground/5">
            <SenderAvatar sender={{ name: sender.name, address: sender.address }} accountId={contactAccountId} size="sm" />
            <span className="min-w-0 flex-1">
              <span className="block truncate font-medium">{label}</span>
              <span className="block text-xs text-muted-foreground">{t('now:cleanupUnread', { count: sender.count })}</span>
            </span>
            <span className="flex shrink-0 items-center gap-1">
              <Button type="button" variant="ghost" size="sm" disabled={busy.has(sender.address)} aria-label={t('now:cleanupMarkReadLabel', { sender: label })} onClick={() => void markRead(sender)}>
                {t('now:cleanupMarkRead')}
              </Button>
              <Button type="button" variant="ghost" size="sm" aria-label={t('now:cleanupViewLabel', { sender: label })} onClick={() => onOpenContact({ address: sender.address, name: sender.name })}>
                {t('now:cleanupView')}
              </Button>
            </span>
          </li>
        )
      })}
    </SenderList>
  )
}
