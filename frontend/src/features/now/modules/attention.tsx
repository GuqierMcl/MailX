import { HugeiconsIcon } from '@hugeicons/react'
import { Alert02Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { cn } from '@/lib/utils'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { rowClass } from '../mail-row-card'
import type { NowModule } from './types'

/** Stopped accounts and sends that failed or may not have gone. It cannot be hidden and always leads its zone. */
export const attentionModule: NowModule = {
  id: 'attention',
  zone: 'act',
  hideable: false,
  title: () => t('now:attention'),
  score: () => 100,
  signature: ({ attention }) => attention.map((item) => item.id).join('|'),
  hasContent: ({ attention }) => attention.length > 0,
  render: ({ data }) => (
    <ul className="flex flex-col rounded-xl bg-muted/40 p-1">
      {data.attention.map((item) => (
        <li key={item.id}>
          <button type="button" onClick={item.onClick} className={rowClass}>
            {item.avatar ? (
              <AccountAvatar displayName={item.avatar.displayName} color={item.avatar.color} providerId={item.avatar.providerId} useProviderIcon={item.avatar.useProviderIcon} className="size-5 text-[10px]" />
            ) : <HugeiconsIcon icon={item.icon} className={cn('size-4 shrink-0 text-muted-foreground', item.tone === 'alert' && 'text-destructive')} />}
            <span className="min-w-0 flex-1 truncate">{item.label}</span>
            {item.avatar && <HugeiconsIcon icon={Alert02Icon} className="size-4 shrink-0 text-destructive" />}
          </button>
        </li>
      ))}
    </ul>
  ),
}
