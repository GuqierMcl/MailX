import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { RefreshIcon } from '@hugeicons/core-free-icons'
import { cn } from 'cn'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import type { Account, AccountSyncState } from '@/generated/accounts'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'
import { t } from '@/i18n'
import { useManualSyncAll } from './use-manual-sync'

export function RefreshAllButton({ accounts, syncStates, onRefresh }: {
  accounts: readonly Pick<Account, 'id' | 'mailboxSourceId'>[]
  syncStates: Record<string, AccountSyncState>
  onRefresh: (accountId: string) => Promise<void>
}) {
  useTranslation()
  const { refreshing, refresh } = useManualSyncAll(accounts, syncStates, onRefresh)
  const turning = useDelayedFlag(refreshing, 400, 700)
  const busy = refreshing || turning
  const label = busy ? t('mail:syncingAllAccounts') : t('mail:syncAllAccounts')
  return (
    <Tooltip>
      <TooltipTrigger
        render={(
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={label}
            aria-disabled={busy || accounts.length === 0}
            aria-busy={busy}
            onClick={() => { if (!busy && accounts.length > 0) refresh() }}
          >
            <HugeiconsIcon
              icon={RefreshIcon}
              strokeWidth={2}
              className={cn(turning && 'animate-spin [animation-duration:1.4s] motion-reduce:animate-none')}
            />
          </Button>
        )}
      />
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
