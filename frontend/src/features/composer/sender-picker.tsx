import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import type { Account } from '@/generated/accounts'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { AccountAvatar } from '@/features/accounts/account-avatar'

/** What recipients see as the sender: `名称 <地址>`, or the address alone. */
function senderLine(account: Account) {
  return account.senderName ? `${account.senderName} <${account.emailAddress}>` : account.emailAddress
}

function Avatar({ account }: { account: Account }) {
  return <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-5 text-[10px]" />
}

/** Avatar, account name, then the address in small muted text; the address gives way first. */
function AccountLine({ account }: { account: Account }) {
  return (
    <span className="flex min-w-0 flex-1 items-center gap-2">
      <Avatar account={account} />
      {/* The name never shrinks while the address can: the address truncates and then disappears first.
          The cap (row minus the avatar) lets the name itself truncate once it is all that is left. */}
      <span className="max-w-[calc(100%-1.75rem)] shrink-0 truncate text-sm">{account.displayName}</span>
      <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{account.emailAddress}</span>
    </span>
  )
}

type SenderPickerProps = {
  accounts: Account[]
  value: string
  disabled?: boolean
  labelId: string
  onChange: (accountId: string) => void
  onFocus?: () => void
}

/**
 * The From row's control (sending.md section 17.1). Accounts are named by their
 * account name; the list adds what recipients will see. With one account there
 * is nothing to choose, so it is plain text in the same layout.
 */
export function SenderPicker({ accounts, value, disabled, labelId, onChange, onFocus }: SenderPickerProps) {
  useTranslation('composer')
  const account = accounts.find((row) => row.id === value) ?? null

  if (account && accounts.length === 1) {
    return (
      <div role="group" aria-label={t('composer:field.fromAccount', { name: account.displayName })} title={senderLine(account)}
        data-slot="composer-sender" className="flex h-6 min-w-0 items-center px-2">
        <AccountLine account={account} />
      </div>
    )
  }

  return (
    <Select
      value={value}
      items={Object.fromEntries(accounts.map((row) => [row.id, row.displayName]))}
      disabled={disabled || accounts.length === 0}
      onValueChange={(next: string | null) => next && onChange(next)}
    >
      <SelectTrigger
        size="sm"
        aria-labelledby={labelId}
        onFocus={onFocus}
        className="min-w-0 max-w-full border-transparent bg-transparent px-2 shadow-none hover:bg-muted/50 dark:bg-transparent dark:hover:bg-muted/50"
      >
        <SelectValue className="min-w-0">{() => (account ? <AccountLine account={account} /> : t('composer:field.chooseSender'))}</SelectValue>
      </SelectTrigger>
      <SelectContent alignItemWithTrigger={false} align="start" className="w-auto min-w-(--anchor-width) max-w-[min(24rem,var(--available-width))]">
        {accounts.map((row) => (
          <SelectItem key={row.id} value={row.id} className="py-1.5 pr-8">
            <span className="flex min-w-0 flex-col gap-0.5">
              <span className="flex min-w-0 items-center gap-2">
                <Avatar account={row} />
                <span className="truncate text-sm">{row.displayName}</span>
              </span>
              <span className="truncate pl-7 text-xs text-muted-foreground">{senderLine(row)}</span>
            </span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
